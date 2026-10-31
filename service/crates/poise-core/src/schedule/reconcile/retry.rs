//! 执行失败的动作隔一会儿再试：同一个进程上同一种动作（优先级、节能、核心、内存与磁盘）连续失败时，
//! 等待从 [`FIRST_MS`] 起逐次加倍，最长 [`MAX_MS`]；成功一次就清零。没权限、进程退出不在这里处理
//! （见 `Reconciler::report`）。

use std::collections::{HashMap, HashSet};
use std::mem::Discriminant;

use super::PlannedAction;
use crate::action::Action;
use crate::model::{Millis, ProcessKey};

/// 第一次失败后等多久再试。
pub(super) const FIRST_MS: Millis = 4_000;
/// 最长等多久。
pub(super) const MAX_MS: Millis = 5 * 60_000;

/// 一种动作在一个进程上连续失败的次数与下次可以再试的时刻。
#[derive(Debug, Clone, Copy)]
struct Backoff {
    failures: u32,
    retry_at: Millis,
}

#[derive(Debug, Default)]
pub(super) struct Retries {
    waiting: HashMap<(ProcessKey, Discriminant<Action>), Backoff>,
}

impl Retries {
    /// 去掉还没到再试时候的动作。
    pub fn drop_waiting(&self, actions: &mut Vec<PlannedAction>, at: Millis) {
        if !self.waiting.is_empty() {
            actions.retain(|a| self.waiting.get(&kind(a)).is_none_or(|b| at >= b.retry_at));
        }
    }

    pub fn failed(&mut self, planned: &PlannedAction, at: Millis) {
        let b = self.waiting.entry(kind(planned)).or_insert(Backoff {
            failures: 0,
            retry_at: at,
        });
        let wait = FIRST_MS.saturating_mul(1 << b.failures.min(16)).min(MAX_MS);
        b.failures += 1;
        b.retry_at = at + wait;
    }

    pub fn succeeded(&mut self, planned: &PlannedAction) {
        self.waiting.remove(&kind(planned));
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.waiting.retain(|(k, _), _| alive.contains(k));
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

fn kind(planned: &PlannedAction) -> (ProcessKey, Discriminant<Action>) {
    (planned.key, std::mem::discriminant(&planned.action))
}
