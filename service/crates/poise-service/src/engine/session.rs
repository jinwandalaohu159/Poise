//! 用户会话：托盘代理上报的会话状态，以及它是否还"新鲜"；界面窗口是否正显示着；
//! 交互用户变化时更新"当前用户"的注册表位置。

use poise_core::model::{Millis, Pid, SessionState};
use poise_platform::settings::user::UserContext;

use super::Engine;
use crate::config::RunMode;

/// 超过这个时间没有上报，就认为看不到用户桌面了。
const FRESH_FOR_MS: Millis = 10_000;
/// 界面显示期间会定期重复上报"可见"；超过这个时间没有上报，视为已经不显示（例如界面意外退出）。
const UI_VISIBLE_FOR_MS: Millis = 30_000;

#[derive(Default)]
pub struct SessionTracker {
    state: SessionState,
    reported_at: Option<Millis>,
    /// 上报会话状态的进程（托盘代理）。
    reporter: Option<Pid>,
    ui_visible_until: Option<Millis>,
}

impl SessionTracker {
    /// 界面代理上报之前，先假定是当前控制台会话。
    pub fn new(console_session: u32) -> Self {
        Self {
            state: SessionState {
                session_id: console_session,
                ..Default::default()
            },
            reported_at: None,
            reporter: None,
            ui_visible_until: None,
        }
    }

    /// 记录一次上报，返回前台进程是否发生了变化。
    pub fn report(&mut self, state: SessionState, reporter: Pid, now: Millis) -> bool {
        let changed = state.foreground_pid != self.state.foreground_pid
            || state.fullscreen != self.state.fullscreen;
        self.state = state;
        self.reported_at = Some(now);
        self.reporter = Some(reporter);
        changed
    }

    /// 某个客户端断开。是上报者（托盘代理）时立即视为过期，返回 true。
    pub fn forget_reporter(&mut self, pid: Pid) -> bool {
        if self.reporter != Some(pid) {
            return false;
        }
        self.reporter = None;
        self.reported_at = None;
        true
    }

    pub fn is_fresh(&self, now: Millis) -> bool {
        self.reported_at
            .is_some_and(|t| now.saturating_sub(t) <= FRESH_FOR_MS)
    }

    /// 当前会话状态。过期时只保留会话 ID，其余信息视为未知。
    pub fn current(&self, now: Millis) -> SessionState {
        if self.is_fresh(now) {
            self.state.clone()
        } else {
            SessionState {
                session_id: self.state.session_id,
                ..Default::default()
            }
        }
    }

    pub fn session_id(&self) -> u32 {
        self.state.session_id
    }

    pub fn set_ui_visible(&mut self, visible: bool, now: Millis) {
        self.ui_visible_until = visible.then_some(now + UI_VISIBLE_FOR_MS);
    }

    /// 界面窗口是否正显示着。
    pub fn ui_visible(&self, now: Millis) -> bool {
        self.ui_visible_until.is_some_and(|until| now <= until)
    }
}

impl Engine {
    /// 交互用户变化（登录、切换用户）时，更新"当前用户"的注册表位置。
    /// 没有它，"资源管理器"等按用户保存的优化项无法读写。
    pub(super) fn on_session_changed(&mut self, session_id: u32) {
        if self.cfg.mode == RunMode::Service {
            let user = UserContext::for_session(session_id);
            if user != self.changes.store.user() {
                log::info!(
                    "交互会话 {session_id}，用户：{:?}",
                    user.as_ref().and_then(|u| u.sid.clone())
                );
            }
            self.changes.store.set_user(user);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_session_drops_foreground_info() {
        let mut t = SessionTracker::default();
        let s = SessionState {
            session_id: 1,
            foreground_pid: Some(5),
            ..Default::default()
        };
        assert!(t.report(s, 7, 0));
        assert_eq!(t.current(5_000).foreground_pid, Some(5));
        assert_eq!(t.current(20_000).foreground_pid, None);
        assert_eq!(t.current(20_000).session_id, 1);
    }

    #[test]
    fn the_reporter_disconnecting_makes_the_session_stale_at_once() {
        let mut t = SessionTracker::default();
        t.report(SessionState::default(), 7, 0);
        assert!(!t.forget_reporter(8));
        assert!(t.is_fresh(1_000));
        assert!(t.forget_reporter(7));
        assert!(!t.is_fresh(1_000));
    }

    #[test]
    fn ui_visibility_expires_without_repeats() {
        let mut t = SessionTracker::default();
        t.set_ui_visible(true, 0);
        assert!(t.ui_visible(10_000));
        assert!(!t.ui_visible(40_000));
        t.set_ui_visible(true, 40_000);
        t.set_ui_visible(false, 41_000);
        assert!(!t.ui_visible(42_000));
    }
}
