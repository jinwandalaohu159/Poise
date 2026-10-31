//! 通过 Windows Core Audio 找出"正在发声"的进程。正在发声的程序不会被降级，也不会被清理。
//!
//! 只开着播放的流不算：很多程序（语音软件、游戏平台、开着页面的浏览器）一直开着一路静音的流。
//! 要听得到声音（会话的音量峰值大于零），并且最近 [`HEARD_FOR`] 内听到过——说话、歌曲之间的停顿不算停。
//! 正在用麦克风的（通话、直播、录音）一直算：没人说话时麦克风也是静的，但程序一直在处理声音。
//!
//! 设备枚举器和各播放设备的会话管理器都是跨进程的 COM 对象，创建一次要几毫秒；心跳每 3 秒查一次，
//! 所以留着复用，只重新枚举会话。播放设备列表每分钟（或者出错时）才重新取一次：插拔耳机后
//! 最迟一分钟内跟上。
//!
//! 复用的会话管理器要先注册过会话通知（[`SessionsChanged`]，什么都不做），之后新开始播放的程序
//! 才会出现在它的会话列表里；否则新程序要等一分钟后重新取会话管理器时才看得到。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use poise_core::model::Pid;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, IAudioSessionNotification, IAudioSessionNotification_Impl,
    IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{Interface, Ref, implement};

/// 播放设备列表多久重新取一次。
const DEVICES_EVERY: Duration = Duration::from_secs(60);
/// 听到声音之后这么久内仍算在发声。
const HEARD_FOR: Duration = Duration::from_secs(15);
/// 峰值超过这个才算有声音（静音流可能有极小的抖动）。
const AUDIBLE_PEAK: f32 = 1e-4;

/// 会话通知：什么都不做，只为让复用的会话管理器跟上新建的会话（见模块说明）。
#[implement(IAudioSessionNotification)]
struct SessionsChanged;

impl IAudioSessionNotification_Impl for SessionsChanged_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        Ok(())
    }
}

/// 一个播放或录音设备的会话管理器，注册着会话通知；丢弃时注销。
struct Manager {
    manager: IAudioSessionManager2,
    notification: Option<IAudioSessionNotification>,
    /// 录音设备（麦克风）。
    capture: bool,
}

impl Manager {
    fn new(manager: IAudioSessionManager2, capture: bool) -> Self {
        let notification: IAudioSessionNotification = SessionsChanged.into();
        // SAFETY: 标准的 COM 调用；通知对象在注销前一直由本对象持有。
        let registered = unsafe { manager.RegisterSessionNotification(&notification) }.is_ok();
        Self {
            manager,
            notification: registered.then_some(notification),
            capture,
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        if let Some(n) = &self.notification {
            // SAFETY: 注销本对象注册的通知。
            let _ = unsafe { self.manager.UnregisterSessionNotification(n) };
        }
    }
}

/// 正在发声的进程。只能在创建它的线程（多线程套间）里使用。
#[derive(Default)]
pub struct AudioSessions {
    enumerator: Option<IMMDeviceEnumerator>,
    /// 各播放设备的会话管理器，以及取得的时间。
    managers: Option<(Vec<Manager>, Instant)>,
    /// 各进程最近一次听到声音的时间。
    heard: HashMap<Pid, Instant>,
}

impl AudioSessions {
    /// 正在发声的进程（见模块说明）。音频服务不可用时返回空。
    pub fn active_pids(&mut self) -> Vec<Pid> {
        match self.collect() {
            Ok(active) => {
                let now = Instant::now();
                for (pid, audible) in &active {
                    if *audible {
                        self.heard.insert(*pid, now);
                    }
                }
                // 流关掉了，或者很久没有声音了：不算。
                self.heard.retain(|pid, at| {
                    active.contains_key(pid) && now.duration_since(*at) < HEARD_FOR
                });
                self.heard.keys().copied().collect()
            }
            Err(_) => {
                // 音频服务重启、设备变了：下次从头取。
                *self = Self::default();
                Vec::new()
            }
        }
    }

    /// 处于活动状态的音频会话所属的进程，以及此刻有没有声音。
    fn collect(&mut self) -> windows::core::Result<HashMap<Pid, bool>> {
        let mut pids = HashMap::new();
        for manager in self.managers()? {
            // SAFETY: 标准的 COM 调用，接口对象由 windows crate 管理引用计数。
            unsafe {
                let sessions = manager.manager.GetSessionEnumerator()?;
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
                        // 读不到峰值时按有声音算（宁可多保护）。录音的流没人说话时也算，但静音了
                        // （峰值恰好是 0，例如后台挂着的通话程序关了麦克风）不算。
                        let peak = control
                            .cast::<IAudioMeterInformation>()
                            .and_then(|m| m.GetPeakValue());
                        let threshold = if manager.capture { 0.0 } else { AUDIBLE_PEAK };
                        let audible = peak.map_or(true, |peak| peak > threshold);
                        *pids.entry(pid).or_insert(false) |= audible;
                    }
                }
            }
        }
        Ok(pids)
    }

    /// 各播放设备的会话管理器；过期了重新取。
    fn managers(&mut self) -> windows::core::Result<&[Manager]> {
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

    fn read_managers(&mut self) -> windows::core::Result<Vec<Manager>> {
        // SAFETY: 标准的 COM 调用，接口对象由 windows crate 管理引用计数。
        unsafe {
            if self.enumerator.is_none() {
                self.enumerator = Some(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?);
            }
            let enumerator = self.enumerator.as_ref().expect("刚刚已创建");
            let mut managers = Vec::new();
            for (flow, capture) in [(eRender, false), (eCapture, true)] {
                let devices = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
                for i in 0..devices.GetCount()? {
                    if let Ok(device) = devices.Item(i)
                        && let Ok(manager) =
                            device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
                    {
                        managers.push(Manager::new(manager, capture));
                    }
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
