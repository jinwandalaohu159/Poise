//! 期望状态与已应用状态的差分：只发出真正需要的动作，并记住原始值以便恢复。

use std::collections::{HashMap, HashSet};

use crate::action::{Action, ExecError, ExecOutcome};
use crate::model::{PriorityClass, ProcessKey};

use super::policy::Desired;

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
}

#[derive(Debug, Default)]
pub struct Reconciler {
    applied: HashMap<ProcessKey, Applied>,
    /// 被拒绝访问的进程（例如反作弊剥夺了句柄权限）：不重试、不绕过。
    blocked: HashSet<ProcessKey>,
}

impl Reconciler {
    pub fn current(&self, key: &ProcessKey) -> Desired {
        self.applied
            .get(key)
            .map(|a| a.current)
            .unwrap_or(Desired::NEUTRAL)
    }

    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    /// 计算让进程达到期望状态所需的动作。读不到原始优先级时不调整优先级。
    pub fn diff(
        &self,
        key: ProcessKey,
        observed_priority: Option<PriorityClass>,
        desired: Desired,
    ) -> Vec<PlannedAction> {
        if self.blocked.contains(&key) {
            return Vec::new();
        }
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
        });
        match &planned.action {
            Action::SetPriority { class, .. } => {
                entry.current.priority = (*class != entry.original_priority).then_some(*class);
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
    }

    /// 恢复所有已调整的进程（服务退出、暂停时调用）。
    pub fn restore_all(&self) -> Vec<PlannedAction> {
        let mut keys: Vec<_> = self.applied.keys().copied().collect();
        keys.sort_by_key(|k| k.pid);
        keys.into_iter()
            .flat_map(|k| self.diff(k, None, Desired::NEUTRAL))
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
        let actions = r.diff(KEY, Some(PriorityClass::Normal), want);
        assert_eq!(actions.len(), 2);
        apply_all(&mut r, &actions);
        assert!(
            r.diff(KEY, Some(PriorityClass::AboveNormal), want)
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
        let actions = r.diff(KEY, Some(PriorityClass::Normal), want);
        apply_all(&mut r, &actions);
        let back = r.diff(KEY, Some(PriorityClass::BelowNormal), Desired::NEUTRAL);
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
        let r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::AboveNormal),
            ..Desired::NEUTRAL
        };
        assert!(r.diff(KEY, Some(PriorityClass::High), want).is_empty());
    }

    #[test]
    fn access_denied_blocks_without_retry() {
        let mut r = Reconciler::default();
        let want = Desired {
            eco: EcoQos::Off,
            ..Desired::NEUTRAL
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), want);
        r.report(&actions[0], &Err(ExecError::AccessDenied));
        assert!(r.diff(KEY, Some(PriorityClass::Normal), want).is_empty());
    }

    #[test]
    fn restore_all_undoes_everything() {
        let mut r = Reconciler::default();
        let want = Desired {
            priority: Some(PriorityClass::High),
            eco: EcoQos::Off,
            cores: CoreSet::All,
        };
        let actions = r.diff(KEY, Some(PriorityClass::Normal), want);
        apply_all(&mut r, &actions);
        let restore = r.restore_all();
        apply_all(&mut r, &restore);
        assert_eq!(r.applied_count(), 0);
    }
}
