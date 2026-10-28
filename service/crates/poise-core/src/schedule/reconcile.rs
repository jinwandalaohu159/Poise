//! 期望状态与已应用状态的差分：只发出真正需要的动作，并记住原始值以便恢复。
//!
//! 程序自己把优先级改回去（自带进程管理的软件、部分游戏）连续达到 [`MAX_FIGHTS`] 次，
//! 本次运行期间就不再调整它：不与程序反复争夺。

use std::collections::{HashMap, HashSet};

use crate::action::{Action, ExecError, ExecOutcome};
use crate::model::{Millis, PriorityClass, ProcessKey};

use super::policy::Desired;

/// 程序连续几次自行改回优先级后放弃调整它。
const MAX_FIGHTS: u8 = 3;

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

    /// 这个进程的优先级是否被 Poise 降低了（它新建的子进程会继承"低于正常"）。
    pub fn lowered(&self, key: &ProcessKey) -> bool {
        self.applied.get(key).is_some_and(|a| {
            a.current
                .priority
                .is_some_and(|p| p.rank() < PriorityClass::Normal.rank())
        })
    }

    /// 计算让进程达到期望状态所需的动作。读不到原始优先级时不调整优先级。
    /// `inherited_low` 表示它的"低于正常"是从被 Poise 降低的父进程继承来的：原始优先级按"正常"记，
    /// 撤销时恢复为正常。
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
        let original = if inherited_low && !self.applied.contains_key(&key) {
            Some(PriorityClass::Normal)
        } else {
            observed_priority
        };
        self.diff_to(key, original, desired)
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
        out
    }

    pub fn report(&mut self, planned: &PlannedAction, result: &Result<ExecOutcome, ExecError>) {
        match result {
            Ok(_) => self.record_success(planned),
            Err(ExecError::AccessDenied) => {
                self.blocked.insert(planned.key);
            }
            Err(ExecError::Gone) => {
                self.applied.remove(&planned.key);
            }
            Err(_) => {}
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
                entry.current.priority = (*class != entry.original_priority).then_some(*class);
                entry.priority_set_at = self.plan_at;
            }
            Action::SetEcoQos { state, .. } => entry.current.eco = *state,
            Action::SetCoreSet { set, .. } => entry.current.cores = *set,
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

/// 提速时不会把原本更高的优先级降下来；降速和恢复则按目标值执行。
fn effective_priority(desired: Option<PriorityClass>, original: PriorityClass) -> PriorityClass {
    match desired {
        None => original,
        Some(p) if p.rank() > PriorityClass::Normal.rank() && original.rank() > p.rank() => {
            original
        }
        Some(p) => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CoreSet, EcoQos};

    const KEY: ProcessKey = ProcessKey { pid: 5, created: 1 };

    fn apply_all(r: &mut Reconciler, actions: &[PlannedAction]) {
        for a in actions {
            r.report(a, &Ok(ExecOutcome::default()));
        }
    }

    #[test]
    fn emits_only_needed_actions_and_converges() {
        let mut r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::AboveNormal),
            eco: EcoQos::Off,
            cores: CoreSet::All,
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
        assert_eq!(actions.len(), 2);
        apply_all(&mut r, &actions);
        assert!(
            r.diff(KEY, Some(PriorityClass::AboveNormal), false, want)
                .is_empty()
        );
    }

    #[test]
    fn returning_to_neutral_restores_original_priority() {
        let mut r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::BelowNormal),
            eco: EcoQos::On,
            cores: CoreSet::Efficiency,
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
        apply_all(&mut r, &actions);
        let back = r.diff(
            KEY,
            Some(PriorityClass::BelowNormal),
            false,
            Desired::NEUTRAL,
        );
        assert!(back.contains(&PlannedAction {
            key: KEY,
            action: Action::SetPriority {
                pid: 5,
                class: PriorityClass::Normal
            },
            original_priority: PriorityClass::Normal,
        }));
        apply_all(&mut r, &back);
        assert_eq!(r.applied_count(), 0);
    }

    #[test]
    fn boost_never_lowers_an_already_higher_priority() {
        let mut r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::AboveNormal),
            ..Desired::NEUTRAL
        };
        assert!(
            r.diff(KEY, Some(PriorityClass::High), false, want)
                .is_empty()
        );
    }

    #[test]
    fn access_denied_blocks_without_retry() {
        let mut r = Reconciler::default();
        let want = Desired {
            eco: EcoQos::Off,
            ..Desired::NEUTRAL
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
        r.report(&actions[0], &Err(ExecError::AccessDenied));
        assert!(
            r.diff(KEY, Some(PriorityClass::Normal), false, want)
                .is_empty()
        );
    }

    #[test]
    fn restore_all_undoes_everything() {
        let mut r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::High),
            eco: EcoQos::Off,
            cores: CoreSet::All,
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
        apply_all(&mut r, &actions);
        let restore = r.restore_all();
        apply_all(&mut r, &restore);
        assert_eq!(r.applied_count(), 0);
    }

    const LOW: Desired = Desired {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    #[test]
    fn gives_up_after_the_program_reverts_its_priority_three_times() {
        let mut r = Reconciler::default();
        r.begin(0);
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, LOW);
        apply_all(&mut r, &actions);
        for at in 1..MAX_FIGHTS as u64 {
            // 程序改回"正常"，Poise 再设一次。
            r.begin(at);
            let again = r.diff(KEY, Some(PriorityClass::Normal), false, LOW);
            assert!(
                again
                    .iter()
                    .any(|a| matches!(a.action, Action::SetPriority { .. }))
            );
            apply_all(&mut r, &again);
        }
        r.begin(MAX_FIGHTS as u64);
        let last = r.diff(KEY, Some(PriorityClass::Normal), false, LOW);
        // 退让：不碰优先级，只把核心范围和节能恢复默认。
        assert!(
            !last
                .iter()
                .any(|a| matches!(a.action, Action::SetPriority { .. }))
        );
        assert_eq!(last.len(), 2);
        apply_all(&mut r, &last);
        assert_eq!(r.state(&KEY), ProcessSchedule::SelfManaged);
        r.begin(10);
        assert!(
            r.diff(KEY, Some(PriorityClass::Normal), false, LOW)
                .is_empty()
        );
    }

    #[test]
    fn stale_snapshot_is_not_a_revert() {
        let mut r = Reconciler::default();
        r.begin(5);
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, LOW);
        apply_all(&mut r, &actions);
        // 立即重新调度时重用的还是设置之前的采样，不算程序改回。
        for _ in 0..5 {
            r.begin(5);
            assert!(
                r.diff(KEY, Some(PriorityClass::Normal), false, LOW)
                    .is_empty()
            );
        }
        assert_eq!(r.state(&KEY), ProcessSchedule::Adjusted(LOW));
    }
}
