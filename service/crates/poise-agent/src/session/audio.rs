//! 通过 Windows Core Audio 找出"正在发声"的进程。正在发声的程序不会被降级，也不会被清理。

use std::collections::HashSet;

use poise_core::model::Pid;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::Interface;

/// 所有播放设备上处于活动状态的音频会话所属的进程。音频服务不可用时返回空。
pub fn active_pids() -> Vec<Pid> {
    collect()
        .map(|s| s.into_iter().collect())
        .unwrap_or_default()
}

fn collect() -> windows::core::Result<HashSet<Pid>> {
    let mut pids = HashSet::new();
    // SAFETY: 标准的 COM 调用，接口对象由 windows crate 管理引用计数。
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
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
