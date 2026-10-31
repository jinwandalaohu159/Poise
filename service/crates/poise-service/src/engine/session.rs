//! 用户会话：托盘代理上报的会话状态，以及它是否还"新鲜"；界面窗口是否正显示着、有没有人在看。
//! 当前用户是谁见 `current_user`。

use std::borrow::Cow;

use poise_core::model::{Millis, Pid, SessionState};

/// 超过这个时间没有上报，就认为看不到用户桌面了。托盘代理最迟每 6 秒上报一次，留出它被卡住、
/// 服务正忙排队的余量，不会因为一次晚到就当成"前台没有程序"。托盘代理真的退出时，连接断开立即可知。
const FRESH_FOR_MS: Millis = 20_000;
/// 界面显示期间会定期重复上报"可见"；超过这个时间没有上报，视为已经不显示（例如界面意外退出）。
const UI_VISIBLE_FOR_MS: Millis = 30_000;
/// 用户这么久没操作（离开了、锁着屏）时，开着的界面不算有人在看（不为它多读传感器、加快采样）。
const UI_UNWATCHED_AFTER_MS: u64 = 2 * 60_000;

#[derive(Default)]
pub struct SessionTracker {
    state: SessionState,
    reported_at: Option<Millis>,
    /// 上报会话状态的进程（托盘代理）。
    reporter: Option<Pid>,
    ui_visible_until: Option<Millis>,
    /// 显示着的界面在哪个会话里（快速切换用户后，另一个用户的界面开着不算有人在看）。
    ui_session: Option<u32>,
    /// 熄屏、待机期间按住上报（见 [`SessionTracker::hold`]）。
    held: bool,
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
            ui_session: None,
            held: false,
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

    /// 这个进程是上报会话状态的托盘代理。
    pub fn is_reporter(&self, pid: Pid) -> bool {
        self.reporter == Some(pid)
    }

    /// 某个客户端断开。是上报者（托盘代理）且它已经退出时立即视为过期，返回 true。进程还在时（一次调用
    /// 超时后断开重连）上报照常按时间过期：重连后接着报就不会暂停，不为一次断线撤销一切又重来。
    pub fn forget_reporter(&mut self, pid: Pid, exited: bool) -> bool {
        if self.reporter != Some(pid) {
            return false;
        }
        self.reporter = None;
        if !exited {
            return false;
        }
        self.reported_at = None;
        self.held = false;
        true
    }

    /// 电脑刚从睡眠里醒来：托盘代理还连着的话，上报当作刚收到（醒来后它要过一会儿才再上报），
    /// 不因为睡着的那段时间暂停、撤销一切又马上重来。
    /// 只顺延睡前那一刻（`slept_at`）还新鲜的上报：托盘代理连着但早就不报了（会话切走、它卡住了）时不算。
    pub fn resumed(&mut self, slept_at: Millis, now: Millis) {
        if self.reporter.is_some() && self.is_fresh(slept_at) {
            self.reported_at = Some(now);
        }
    }

    /// 熄屏、进入待机（见 `standby`）：托盘代理随后可能被系统冻结，报不上来不等于退出了。它连着、那一刻
    /// 上报还新鲜的话，按住上报不让它过期，直到 [`SessionTracker::release`]；它真的退出了（连接断开、
    /// 进程结束）照样立即过期。
    pub fn hold(&mut self, now: Millis) {
        self.held = self.reporter.is_some() && self.is_fresh(now);
    }

    /// 亮屏、醒来：放开按住的上报，当作刚收到（托盘代理要过一会儿才再上报）。
    pub fn release(&mut self, now: Millis) {
        if std::mem::take(&mut self.held) {
            self.reported_at = Some(now);
        }
    }

    pub fn is_fresh(&self, now: Millis) -> bool {
        self.held
            || self
                .reported_at
                .is_some_and(|t| now.saturating_sub(t) <= FRESH_FOR_MS)
    }

    /// 当前会话状态（不复制，每轮要取好几次）。过期时只保留会话 ID，其余信息视为未知。
    pub fn current(&self, now: Millis) -> Cow<'_, SessionState> {
        if self.is_fresh(now) {
            Cow::Borrowed(&self.state)
        } else {
            Cow::Owned(SessionState {
                session_id: self.state.session_id,
                ..Default::default()
            })
        }
    }

    pub fn session_id(&self) -> u32 {
        self.state.session_id
    }

    /// `session`：界面所在的会话（查不到时为 None，不限制）。
    pub fn set_ui_visible(&mut self, visible: bool, session: Option<u32>, now: Millis) {
        self.ui_visible_until = visible.then_some(now + UI_VISIBLE_FOR_MS);
        self.ui_session = session;
    }

    /// 界面窗口正显示着、有人在看：在用户正在用的会话里，用户也没离开。
    pub fn ui_visible(&self, now: Millis) -> bool {
        let shown = self.ui_visible_until.is_some_and(|until| now <= until);
        let here = self.ui_session.is_none_or(|s| s == self.state.session_id);
        let present = !self.is_fresh(now) || self.state.idle_ms < UI_UNWATCHED_AFTER_MS;
        shown && here && present
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
        assert_eq!(t.current(FRESH_FOR_MS).foreground_pid, Some(5));
        assert_eq!(t.current(FRESH_FOR_MS + 1).foreground_pid, None);
        assert_eq!(t.current(FRESH_FOR_MS + 1).session_id, 1);
    }

    #[test]
    fn the_reporter_exiting_makes_the_session_stale_at_once() {
        let mut t = SessionTracker::default();
        t.report(SessionState::default(), 7, 0);
        assert!(!t.forget_reporter(8, true));
        assert!(t.is_fresh(1_000));
        assert!(t.forget_reporter(7, true));
        assert!(!t.is_fresh(1_000));
    }

    #[test]
    fn a_reporter_that_only_dropped_the_connection_expires_on_time() {
        let mut t = SessionTracker::default();
        t.report(SessionState::default(), 7, 0);
        assert!(!t.forget_reporter(7, false));
        assert!(t.is_fresh(FRESH_FOR_MS));
        assert!(!t.is_fresh(FRESH_FOR_MS + 1));
        // 重连后接着报：一直新鲜。
        t.report(SessionState::default(), 7, 5_000);
        assert!(t.is_fresh(FRESH_FOR_MS + 1));
    }

    #[test]
    fn waking_up_only_extends_a_report_that_was_fresh_before_sleeping() {
        let mut t = SessionTracker::default();
        t.report(SessionState::default(), 7, 0);
        t.resumed(10_000, 3_600_000);
        assert!(t.is_fresh(3_600_000 + 1_000));
        // 睡前就已经很久没报了（会话切走了）：醒来不当成新鲜。
        let mut stale = SessionTracker::default();
        stale.report(SessionState::default(), 7, 0);
        stale.resumed(60_000, 3_600_000);
        assert!(!stale.is_fresh(3_600_000));
    }

    #[test]
    fn a_frozen_reporter_stays_fresh_until_the_screen_comes_back() {
        let mut t = SessionTracker::default();
        t.report(SessionState::default(), 7, 0);
        t.hold(5_000);
        // 待机一小时，托盘代理一直没报。
        assert!(t.is_fresh(3_600_000));
        t.release(3_600_000);
        assert!(t.is_fresh(3_600_000 + FRESH_FOR_MS));
        assert!(!t.is_fresh(3_600_000 + FRESH_FOR_MS + 1));
        // 熄屏时已经过期的不按住；按住期间托盘代理退出了，立即过期。
        let mut stale = SessionTracker::default();
        stale.report(SessionState::default(), 7, 0);
        stale.hold(FRESH_FOR_MS + 1);
        assert!(!stale.is_fresh(FRESH_FOR_MS + 1));
        let mut gone = SessionTracker::default();
        gone.report(SessionState::default(), 7, 0);
        gone.hold(1_000);
        assert!(gone.forget_reporter(7, true));
        assert!(!gone.is_fresh(2_000));
    }

    #[test]
    fn ui_visibility_expires_without_repeats() {
        let mut t = SessionTracker::default();
        t.set_ui_visible(true, None, 0);
        assert!(t.ui_visible(10_000));
        assert!(!t.ui_visible(40_000));
        t.set_ui_visible(true, None, 40_000);
        t.set_ui_visible(false, None, 41_000);
        assert!(!t.ui_visible(42_000));
    }

    #[test]
    fn a_window_nobody_is_looking_at_is_not_watched() {
        let mut t = SessionTracker::new(1);
        // 另一个用户会话里开着的界面。
        t.report(
            SessionState {
                session_id: 1,
                ..Default::default()
            },
            7,
            0,
        );
        t.set_ui_visible(true, Some(2), 0);
        assert!(!t.ui_visible(1_000));
        t.set_ui_visible(true, Some(1), 0);
        assert!(t.ui_visible(1_000));
        // 用户离开了（锁着屏）。
        t.report(
            SessionState {
                session_id: 1,
                idle_ms: UI_UNWATCHED_AFTER_MS,
                ..Default::default()
            },
            7,
            1_000,
        );
        assert!(!t.ui_visible(2_000));
    }
}
