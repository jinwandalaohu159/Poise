//! 采集用户会话的状态（只读），上报给服务。服务在会话 0，看不到用户桌面，全靠这里提供。

pub mod audio;
pub mod desktop;

use poise_core::model::{Pid, SessionState};

/// 采集一次会话状态。`audio` 为 None 时重新枚举音频会话（相对较贵），否则沿用给定的结果。
pub fn capture(audio: Option<&[Pid]>) -> SessionState {
    let foreground = desktop::foreground_window();
    SessionState {
        session_id: current_session(),
        foreground_pid: desktop::pid_of(foreground),
        fullscreen: desktop::is_fullscreen(foreground),
        visible_window_pids: desktop::visible_window_pids(),
        audio_pids: audio.map_or_else(audio::active_pids, <[Pid]>::to_vec),
        shell_pid: desktop::shell_pid(),
        idle_ms: desktop::idle_ms(),
    }
}

fn current_session() -> u32 {
    let mut session = 0;
    // SAFETY: 输出到本地变量。
    let _ = unsafe {
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            std::process::id(),
            &mut session,
        )
    };
    session
}
