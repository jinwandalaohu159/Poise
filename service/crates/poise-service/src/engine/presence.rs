//! Poise 只在托盘代理运行时起作用：用户退出 Poise（托盘代理退出）的那一刻，撤销所有进程调整、
//! 还原散热与电源计划，就像 Poise 不存在一样；托盘代理重新连上（下次登录、重新打开）后从头开始。
//! 暂停期间服务只做只读的采样，不调度、不切换模式、不自动清理。

use poise_core::model::{Millis, Pid};
use poise_protocol::Event;

use super::Engine;

/// 暂停期间还有没撤销成的电源设置（撤销时电源服务出错，放回了变更日志）时，隔这么久再撤销一次。
const RETRY_WITHDRAW_MS: Millis = 60_000;

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
            self.retry_withdraw(now);
            false
        }
    }

    /// 暂停着、却还有 Poise 的电源设置没撤销成：隔一阵再撤销一次（Poise 退出了就该像不存在一样）。
    fn retry_withdraw(&mut self, now: Millis) {
        let left = self.changes.journal().owns(poise_hardware::power::OWNER);
        if !left || now.saturating_sub(self.withdraw_retried_at) < RETRY_WITHDRAW_MS {
            return;
        }
        self.withdraw_retried_at = now;
        log::info!("还有没撤销成的电源设置，再撤销一次");
        self.restore_hardware();
    }

    /// 某个客户端断开。是托盘代理、而且它已经退出时立即暂停，不等上报过期；它还在（一次调用超时后
    /// 断开重连）时按上报是否过期判断，重连后接着报就不暂停。
    pub(super) fn on_client_gone(&mut self, pid: Pid, exited: bool) {
        let reporter = self.session.is_reporter(pid);
        if self.session.forget_reporter(pid, exited) {
            self.pause();
        } else if reporter {
            log::info!("托盘代理断开了连接（进程还在），等它重连");
        }
    }

    fn pause(&mut self) {
        if !self.paused {
            self.withdraw(Withdraw::Pause);
            self.logbook.event(Event::Paused);
        }
    }

    /// 撤销 Poise 的全部运行时影响：进程调整、散热与电源计划、风扇。系统优化项保留（它们是用户选择的）。
    /// 暂停和服务停止共用。已经暂停着时，暂停不再重复；服务停止、关机时照样再撤销一次（和 0.7.0 一样）：
    /// 暂停时那次可能没做成（散热接口一时不可用、采样失败），上次异常退出留下的电源设置也还在日志里，
    /// 停止之后就没有下次了。撤销是按记录做的，已经撤销过的不会再动。
    pub(super) fn withdraw(&mut self, why: Withdraw) {
        let first = !self.paused;
        if !first && why == Withdraw::Pause {
            return;
        }
        self.paused = true;
        // 暂停期间不切换模式：不再等预读、不再按失败退避（否则一直按等预读的节奏采样）。
        self.mode_attempts.settled();
        if first {
            self.logbook.end_session();
            self.scheduler.reset_tracking();
        }
        if why == Withdraw::SystemShutdown {
            let now = self.now();
            self.adjusted.flush(Vec::new(), now);
        } else {
            self.release_processes();
        }
        self.restore_hardware();
        if why != Withdraw::Pause {
            self.settle_hardware();
        }
    }
}
