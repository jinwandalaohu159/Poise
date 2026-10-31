//! 期望状态与已应用状态的差分：只发出真正需要的动作，并记住原始值以便恢复。
//!
//! 程序自己把优先级改回去（自带进程管理的软件、部分游戏）连续达到 [`MAX_FIGHTS`] 次，
//! 本次运行期间就不再调整它：不与程序反复争夺。
//!
//! 执行失败（不是没权限、不是进程退出）的动作隔一会儿再试，间隔逐次加倍（见 [`retry`]）：
//! 不每轮都重复同一个注定失败的调用。

mod retry;

use std::collections::{HashMap, HashSet};

use crate::action::{Action, ExecError, ExecOutcome};
use crate::model::{Millis, PriorityClass, ProcessKey};

use super::desired::Desired;
use super::leftovers::Adjusted;
use retry::Retries;

/// 程序连续几次自行改回优先级后放弃调整它。
const MAX_FIGHTS: u8 = 3;
/// 父进程恢复后这么久内新出现的子进程，仍可能是在恢复前继承了"低于正常"。
const INHERIT_GRACE_MS: Millis = 10_000;

/// 一个进程当前的调度状态（给界面看）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSchedule {
    /// 按系统默认运行（没有调整，或期望状态就是默认）。
    Default,
    /// 已调整为这个状态。
    Adjusted(Desired),
    /// 程序自行管理优先级，Poise 已经退让。
    SelfManaged,
    /// 系统拒绝访问（受保护或被反作弊保护）。
    Denied,
}

/// 一个准备执行的调度动作，附带安全守卫需要的原始优先级。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub key: ProcessKey,
    pub action: Action,
    pub original_priority: PriorityClass,
}

#[derive(Debug, Clone, Copy)]
struct Applied {
    original_priority: PriorityClass,
    current: Desired,
    /// 最近一次设置优先级时所依据的采样时刻；之后的采样才能反映这次设置。
    priority_set_at: Millis,
}

#[derive(Debug, Default)]
pub struct Reconciler {
    applied: HashMap<ProcessKey, Applied>,
    /// 被拒绝访问的进程（例如反作弊剥夺了句柄权限）：不重试、不绕过。
    blocked: HashSet<ProcessKey>,
    /// 程序自行改回优先级的连续次数。
    fights: HashMap<ProcessKey, u8>,
    /// 已经退让、本次运行期间不再调整的进程。
    self_managed: HashSet<ProcessKey>,
    /// 刚从"低于正常"恢复的进程及恢复时刻（见 [`INHERIT_GRACE_MS`]）。
    restored_low: HashMap<ProcessKey, Millis>,
    /// 执行失败、暂时不再试的动作。
    retries: Retries,
    /// 本轮计划所依据的采样时刻。
    plan_at: Millis,
}

impl Reconciler {
    /// 每轮计划开始时调用，`at` 是这轮所依据的采样时刻。
    pub fn begin(&mut self, at: Millis) {
        self.plan_at = at;
    }

    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    pub fn state(&self, key: &ProcessKey) -> ProcessSchedule {
        if self.self_managed.contains(key) {
            ProcessSchedule::SelfManaged
        } else if self.blocked.contains(key) {
            ProcessSchedule::Denied
        } else {
            self.applied.get(key).map_or(ProcessSchedule::Default, |a| {
                ProcessSchedule::Adjusted(a.current)
            })
        }
    }

    /// 进程在 Poise 调整之前的优先级；没调整过时为 None。
    pub fn original_priority(&self, key: &ProcessKey) -> Option<PriorityClass> {
        self.applied.get(key).map(|a| a.original_priority)
    }

    /// 这个进程的优先级是否被 Poise 降低了（它新建的子进程会继承"低于正常"）。
    /// 刚恢复不久的也算：恢复之前新建的子进程已经继承了"低于正常"。
    pub fn lowered(&self, key: &ProcessKey) -> bool {
        let now_low = self.applied.get(key).is_some_and(|a| {
            a.current
                .priority
                .is_some_and(|p| p.rank() < PriorityClass::Normal.rank())
        });
        let recently_low = self
            .restored_low
            .get(key)
            .is_some_and(|t| self.plan_at.saturating_sub(*t) <= INHERIT_GRACE_MS);
        now_low || recently_low
    }

    /// 计算让进程达到期望状态所需的动作。读不到原始优先级时不调整优先级。
    /// `inherited_low` 表示它的"低于正常"是从被 Poise 降低的父进程继承来的：原始优先级按"正常"记，
    /// 不需要让路或撤销时恢复为正常。
    pub fn diff(
        &mut self,
        key: ProcessKey,
        observed_priority: Option<PriorityClass>,
        inherited_low: bool,
        desired: Desired,
    ) -> Vec<PlannedAction> {
        if self.blocked.contains(&key) || self.self_managed.contains(&key) {
            return Vec::new();
        }
        if self.priority_reverted(key, observed_priority) {
            return self.give_up(key);
        }
        if inherited_low && !self.applied.contains_key(&key) {
            // 继承来的"低于正常"是 Poise 间接造成的：记为 Poise 的调整（原本是正常），
            // 不需要让路时恢复，Poise 退出时一并还原。
            self.applied.insert(
                key,
                Applied {
                    original_priority: PriorityClass::Normal,
                    current: Desired {
                        priority: Some(PriorityClass::BelowNormal),
                        ..Desired::NEUTRAL
                    },
                    priority_set_at: self.plan_at,
                },
            );
        }
        let mut actions = self.diff_to(key, observed_priority, desired);
        self.retries.drop_waiting(&mut actions, self.plan_at);
        actions
    }

    /// 程序是否又一次自己改回了优先级，并且已经达到放弃的次数。没达到时按实际优先级记录，
    /// 随后的差分会重新设置一次。只看设置之后的采样：重用旧采样（立即重新调度）时还看不到刚才的设置。
    fn priority_reverted(&mut self, key: ProcessKey, observed: Option<PriorityClass>) -> bool {
        let Some(applied) = self.applied.get_mut(&key) else {
            return false;
        };
        let (Some(set), Some(observed)) = (applied.current.priority, observed) else {
            return false;
        };
        if self.plan_at <= applied.priority_set_at {
            return false;
        }
        if observed == set {
            self.fights.remove(&key);
            return false;
        }
        applied.current.priority = (observed != applied.original_priority).then_some(observed);
        let fights = self.fights.entry(key).or_default();
        *fights += 1;
        *fights >= MAX_FIGHTS
    }

    /// 退让：不再管它的优先级（程序自己在管），核心范围与节能恢复默认。
    fn give_up(&mut self, key: ProcessKey) -> Vec<PlannedAction> {
        self.self_managed.insert(key);
        self.fights.remove(&key);
        let Some(applied) = self.applied.get_mut(&key) else {
            return Vec::new();
        };
        applied.current.priority = None;
        if applied.current.is_neutral() {
            self.applied.remove(&key);
            return Vec::new();
        }
        self.diff_to(key, None, Desired::NEUTRAL)
    }

    fn diff_to(
        &self,
        key: ProcessKey,
        observed_priority: Option<PriorityClass>,
        desired: Desired,
    ) -> Vec<PlannedAction> {
        let applied = self.applied.get(&key);
        let current = applied.map(|a| a.current).unwrap_or(Desired::NEUTRAL);
        let original = applied.map(|a| a.original_priority).or(observed_priority);
        let pid = key.pid;
        let mut out = Vec::new();
        let mut push = |action, original_priority| {
            out.push(PlannedAction {
                key,
                action,
                original_priority,
            })
        };

        if let Some(original) = original {
            let target = effective_priority(desired.priority, original);
            let now = effective_priority(current.priority, original);
            if target != now {
                push(Action::SetPriority { pid, class: target }, original);
            }
        }
        let original = original.unwrap_or_default();
        if desired.eco != current.eco {
            push(
                Action::SetEcoQos {
                    pid,
                    state: desired.eco,
                },
                original,
            );
        }
        if desired.cores != current.cores {
            push(
                Action::SetCoreSet {
                    pid,
                    set: desired.cores,
                },
                original,
            );
        }
        if desired.resources != current.resources {
            push(
                Action::SetResourcePriority {
                    pid,
                    priority: desired.resources,
                },
                original,
            );
        }
        out
    }

    pub fn report(&mut self, planned: &PlannedAction, result: &Result<ExecOutcome, ExecError>) {
        match result {
            Ok(_) => {
                self.retries.succeeded(planned);
                self.record_success(planned);
            }
            Err(ExecError::AccessDenied) => {
                self.blocked.insert(planned.key);
            }
            Err(ExecError::Gone) => {
                self.applied.remove(&planned.key);
            }
            Err(ExecError::Failed(_) | ExecError::Denied(_)) => {
                self.retries.failed(planned, self.plan_at);
            }
        }
    }

    fn record_success(&mut self, planned: &PlannedAction) {
        let entry = self.applied.entry(planned.key).or_insert(Applied {
            original_priority: planned.original_priority,
            current: Desired::NEUTRAL,
            priority_set_at: 0,
        });
        match &planned.action {
            Action::SetPriority { class, .. } => {
                let was_low = entry
                    .current
                    .priority
                    .is_some_and(|p| p.rank() < PriorityClass::Normal.rank());
                if was_low && class.rank() >= PriorityClass::Normal.rank() {
                    self.restored_low.insert(planned.key, self.plan_at);
                }
                entry.current.priority = (*class != entry.original_priority).then_some(*class);
                entry.priority_set_at = self.plan_at;
            }
            Action::SetEcoQos { state, .. } => entry.current.eco = *state,
            Action::SetCoreSet { set, .. } => entry.current.cores = *set,
            Action::SetResourcePriority { priority, .. } => entry.current.resources = *priority,
            _ => {}
        }
        if entry.current.is_neutral() {
            self.applied.remove(&planned.key);
        }
    }

    /// 进程退出后清理记录。
    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.applied.retain(|k, _| alive.contains(k));
        self.blocked.retain(|k| alive.contains(k));
        self.fights.retain(|k, _| alive.contains(k));
        self.self_managed.retain(|k| alive.contains(k));
        self.restored_low.retain(|k, _| alive.contains(k));
        self.retries.retain_alive(alive);
    }

    /// 所有已调整的进程（按进程号排序，便于比较有没有变化）。
    pub fn adjusted(&self) -> Vec<Adjusted> {
        let mut out: Vec<Adjusted> = self
            .applied
            .iter()
            .map(|(key, a)| Adjusted {
                key: *key,
                original_priority: a.original_priority,
            })
            .collect();
        out.sort_by_key(|a| (a.key.pid, a.key.created));
        out
    }

    /// 恢复所有已调整的进程（服务停止时调用）。
    pub fn restore_all(&self) -> Vec<PlannedAction> {
        let mut keys: Vec<_> = self.applied.keys().copied().collect();
        keys.sort_by_key(|k| k.pid);
        keys.into_iter()
            .flat_map(|k| self.diff_to(k, None, Desired::NEUTRAL))
            .collect()
    }
}

/// 能提到的最高优先级（只给用户标记的游戏）。再高会和声音、输入、系统组件抢资源。
const RAISE_CAP: PriorityClass = PriorityClass::AboveNormal;

/// 实际要设置的优先级：
/// - 让路：目标比原来低时才降（例如用户自己设为"低"的程序，不会因为"让路"反而被调到"低于正常"）；
/// - 提速：最多到"高于正常"；原来已经更高（程序或用户自己设的）就保持原样，不会反而被降下来。
fn effective_priority(desired: Option<PriorityClass>, original: PriorityClass) -> PriorityClass {
    let normal = PriorityClass::Normal.rank();
    match desired {
        Some(p) if p.rank() < normal && p.rank() < original.rank() => p,
        Some(p)
            if p.rank() > normal && p.rank() <= RAISE_CAP.rank() && p.rank() > original.rank() =>
        {
            p
        }
        _ => original,
    }
}

#[cfg(test)]
mod tests;
