//! 通过 Windows Core Audio 找出"正在发声"的进程。正在发声的程序不会被降级，也不会被清理。
//!
//! 设备枚举器和各播放设备的会话管理器都是跨进程的 COM 对象，创建一次要几毫秒；心跳每 3 秒查一次，
//! 所以留着复用，只重新枚举会话。播放设备列表每分钟（或者出错时）才重新取一次：插拔耳机后
//! 最迟一分钟内跟上。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use poise_core::model::Pid;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::Interface;

/// 播放设备列表多久重新取一次。
const DEVICES_EVERY: Duration = Duration::from_secs(60);

/// 正在发声的进程。只能在创建它的线程（多线程套间）里使用。
#[derive(Default)]
pub struct AudioSessions {
    enumerator: Option<IMMDeviceEnumerator>,
    /// 各播放设备的会话管理器，以及取得的时间。
    managers: Option<(Vec<IAudioSessionManager2>, Instant)>,
}

impl AudioSessions {
    /// 所有播放设备上处于活动状态的音频会话所属的进程。音频服务不可用时返回空。
    pub fn active_pids(&mut self) -> Vec<Pid> {
        match self.collect() {
            Ok(pids) => pids.into_iter().collect(),
            Err(_) => {
                // 音频服务重启、设备变了：下次从头取。
                *self = Self::default();
                Vec::new()
            }
        }
    }

    fn collect(&mut self) -> windows::core::Result<HashSet<Pid>> {
        let mut pids = HashSet::new();
        for manager in self.managers()? {
            // SAFETY: 标准的 COM 调用，接口对象由 windows crate 管理引用计数。
            unsafe {
                let sessions = manager.GetSessionEnumerator()?;
                for j in 0..sessions.GetCount()? {
                    let Ok(control) = sessions.GetSession(j) else {
                        continue;
                    };
                    let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                        continue;
                    };
                    if control.GetState() == Ok(AudioSessionStateActive)
                        && let Ok(pid) = control.GetProcessId()
                        && pid != 0
                    {
                        pids.insert(pid);
                    }
                }
            }
        }
        Ok(pids)
    }

    /// 各播放设备的会话管理器；过期了重新取。
    fn managers(&mut self) -> windows::core::Result<&[IAudioSessionManager2]> {
        let fresh = self
            .managers
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < DEVICES_EVERY);
        if !fresh {
            let list = self.read_managers()?;
            self.managers = Some((list, Instant::now()));
        }
        Ok(self.managers.as_ref().map_or(&[], |(m, _)| m.as_slice()))
    }

    fn read_managers(&mut self) -> windows::core::Result<Vec<IAudioSessionManager2>> {
        // SAFETY: 标准的 COM 调用，接口对象由 windows crate 管理引用计数。
        unsafe {
            if self.enumerator.is_none() {
                self.enumerator = Some(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?);
            }
            let enumerator = self.enumerator.as_ref().expect("刚刚已创建");
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let mut managers = Vec::new();
            for i in 0..devices.GetCount()? {
                if let Ok(device) = devices.Item(i)
                    && let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
                {
                    managers.push(manager);
                }
            }
            Ok(managers)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// 复用会话管理器之后，每次查询应当便宜得多。
    /// 手动运行：cargo test --release -p poise-agent audio -- --ignored --nocapture
    #[test]
    #[ignore]
    fn measure_repeated_queries() {
        // SAFETY: 测试线程加入多线程套间。
        let _ = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        };
        let t = Instant::now();
        for _ in 0..20 {
            std::hint::black_box(AudioSessions::default().active_pids());
        }
        let fresh = t.elapsed().as_micros() / 20;
        let mut cached = AudioSessions::default();
        let first = cached.active_pids();
        let t = Instant::now();
        for _ in 0..20 {
            assert_eq!(cached.active_pids().len(), first.len());
        }
        let reused = t.elapsed().as_micros() / 20;
        println!("每次新建 {fresh} 微秒，复用 {reused} 微秒");
    }
}
