//! 采集用户会话的状态（只读），上报给服务。服务在会话 0，看不到用户桌面，全靠这里提供。

pub mod audio;
pub mod desktop;

use poise_core::model::{Pid, SessionState};

/// 采集会话状态。记着上一次的前台：切换过程中的过渡窗口（Alt+Tab、任务栏）、托盘代理自己的菜单
/// 成为前台时沿用它——否则每次切换都会多报一次"前台换了"又换回来，服务跟着多调度一次。
#[derive(Default)]
pub struct Capturer {
    foreground: Option<Pid>,
    /// 沿用的前台程序的窗口：它已经关掉时不再沿用（关掉游戏后前台一时没有窗口，不能一直当它还在前台）。
    window: isize,
    fullscreen: bool,
    session_id: Option<u32>,
}

impl Capturer {
    /// 采集一次。`audio` 是正在发声的进程（见 [`audio::AudioSessions`]，由调用方决定什么时候重新枚举）。
    /// 进程列表排好序：窗口层叠顺序变了不算状态变化（服务只关心有哪些进程）。
    pub fn capture(&mut self, audio: &[Pid]) -> SessionState {
        let session_id = *self.session_id.get_or_insert_with(current_session);
        if desktop::desktop_away() {
            // 锁屏、屏保、切换了用户：没有前台程序，也不算全屏（锁屏界面铺满屏幕，但没人在用）。
            self.foreground = None;
            self.fullscreen = false;
        } else {
            let window = desktop::foreground_window();
            let pid = desktop::app_pid(window);
            let passing = window.is_invalid()
                || pid == Some(std::process::id())
                || desktop::is_transient(window);
            if !passing {
                self.foreground = pid;
                self.window = window.0 as isize;
                self.fullscreen = desktop::is_fullscreen(window);
            } else if !desktop::window_exists(windows::Win32::Foundation::HWND(self.window as _)) {
                self.foreground = None;
                self.fullscreen = false;
            }
        }
        SessionState {
            session_id,
            foreground_pid: self.foreground,
            fullscreen: self.fullscreen,
            visible_window_pids: sorted(desktop::visible_window_pids()),
            audio_pids: sorted(audio.to_vec()),
            shell_pid: desktop::shell_pid(),
            idle_ms: desktop::idle_ms(),
        }
    }
}

/// 除了"多久没操作"之外是否相同（它每次都不同，靠定期上报更新即可）。
pub fn same_desktop(a: &SessionState, b: &SessionState) -> bool {
    a.session_id == b.session_id
        && a.foreground_pid == b.foreground_pid
        && a.fullscreen == b.fullscreen
        && a.shell_pid == b.shell_pid
        && a.visible_window_pids == b.visible_window_pids
        && a.audio_pids == b.audio_pids
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_time_alone_is_not_a_change() {
        let a = SessionState {
            foreground_pid: Some(4),
            idle_ms: 10,
            ..Default::default()
        };
        let b = SessionState {
            idle_ms: 9_000,
            ..a.clone()
        };
        assert!(same_desktop(&a, &b));
        let c = SessionState {
            fullscreen: true,
            ..a.clone()
        };
        assert!(!same_desktop(&a, &c));
    }
}
