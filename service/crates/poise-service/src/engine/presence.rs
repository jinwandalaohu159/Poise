//! Poise 只在托盘代理运行时起作用：用户退出 Poise（托盘代理退出）的那一刻，撤销所有进程调整、
//! 还原散热与电源计划，就像 Poise 不存在一样；托盘代理重新连上（下次登录、重新打开）后从头开始。
//! 暂停期间服务只做只读的采样，不调度、不切换模式、不自动清理。

use poise_core::model::{Millis, Pid};
use poise_protocol::Event;

use super::Engine;

impl Engine {
    /// 托盘代理是否在线（在按时上报会话状态）。离线时进入暂停。返回现在是否在起作用。
    pub(super) fn ensure_presence(&mut self, now: Millis) -> bool {
        if self.session.is_fresh(now) {
            if self.paused {
                self.paused = false;
                self.logbook.event(Event::Resumed);
            }
            true
        } else {
            self.pause();
            false
        }
    }

    /// 某个客户端断开。是托盘代理时立即暂停，不等上报过期。
    pub(super) fn on_client_gone(&mut self, pid: Pid) {
        if self.session.forget_reporter(pid) {
            self.pause();
        }
    }

    /// 撤销 Poise 的全部运行时影响：进程调整、散热与电源计划、风扇。系统优化项保留（它们是用户选择的）。
    fn pause(&mut self) {
        if self.paused {
            return;
        }
        self.paused = true;
        self.logbook.end_session();
        self.release_processes();
        self.scheduler.reset_tracking();
        self.restore_hardware();
        self.logbook.event(Event::Paused);
    }
}
