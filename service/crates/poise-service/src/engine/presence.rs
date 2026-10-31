//! Poise 只在托盘代理运行时起作用：用户退出 Poise（托盘代理退出）的那一刻，撤销所有进程调整、
//! 还原散热与电源计划，就像 Poise 不存在一样；托盘代理重新连上（下次登录、重新打开）后从头开始。
//! 暂停期间服务只做只读的采样，不调度、不切换模式、不自动清理。

use poise_core::model::{Millis, Pid};
use poise_protocol::Event;

use super::Engine;

/// 为什么撤销。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Withdraw {
    /// 托盘代理不在了（Poise 退出）：服务还在跑，不等硬件还原完成。
    Pause,
    /// 服务停止：等硬件还原完成。
    Stop,
    /// 系统关机：进程都要结束了，不逐个撤销进程调整（只清空调整记录）；等硬件还原完成。
    SystemShutdown,
}

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

    fn pause(&mut self) {
        if !self.paused {
            self.withdraw(Withdraw::Pause);
            self.logbook.event(Event::Paused);
        }
    }

    /// 撤销 Poise 的全部运行时影响：进程调整、散热与电源计划、风扇。系统优化项保留（它们是用户选择的）。
    /// 暂停和服务停止共用；已经撤销过（暂停中）就不再重复——还原散热模式每次都要调用机型接口——
    /// 服务停止时只等暂停时交给硬件执行线程的还原做完。
    pub(super) fn withdraw(&mut self, why: Withdraw) {
        let wait = why != Withdraw::Pause;
        if self.paused {
            if wait {
                self.settle_hardware();
            }
            return;
        }
        self.paused = true;
        self.logbook.end_session();
        if why == Withdraw::SystemShutdown {
            let now = self.now();
            self.adjusted.flush(Vec::new(), now);
        } else {
            self.release_processes();
        }
        self.scheduler.reset_tracking();
        self.restore_hardware();
        if wait {
            self.settle_hardware();
        }
    }
}
