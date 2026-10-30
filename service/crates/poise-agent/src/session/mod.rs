//! 采集用户会话的状态（只读），上报给服务。服务在会话 0，看不到用户桌面，全靠这里提供。

pub mod audio;
pub mod desktop;

use poise_core::model::{Pid, SessionState};

/// 采集一次会话状态。`audio` 是正在发声的进程（见 [`audio::AudioSessions`]，由调用方决定什么时候重新枚举）。
/// 进程列表排好序：窗口层叠顺序变了不算状态变化（服务只关心有哪些进程）。
pub fn capture(audio: &[Pid]) -> SessionState {
    let foreground = desktop::foreground_window();
    SessionState {
        session_id: current_session(),
        foreground_pid: desktop::app_pid(foreground),
        fullscreen: desktop::is_fullscreen(foreground),
        visible_window_pids: sorted(desktop::visible_window_pids()),
        audio_pids: sorted(audio.to_vec()),
        shell_pid: desktop::shell_pid(),
        idle_ms: desktop::idle_ms(),
    }
}

/// 除了"多久没操作"之外是否相同（它每次都不同，靠定期上报更新即可）。
pub fn same_desktop(a: &SessionState, b: &SessionState) -> bool {
    SessionState {
        idle_ms: 0,
        ..a.clone()
    } == SessionState {
        idle_ms: 0,
        ..b.clone()
    }
}

fn sorted(mut pids: Vec<Pid>) -> Vec<Pid> {
    pids.sort_unstable();
    pids.dedup();
    pids
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
