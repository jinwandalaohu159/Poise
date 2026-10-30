//! 上次运行留下的调整：服务把调整过的进程存到磁盘上，异常退出（崩溃、被强行结束）来不及撤销时，
//! 下次启动按记录把还在运行的进程恢复原样。

use serde::{Deserialize, Serialize};

use super::reconcile::PlannedAction;
use crate::action::{Action, CoreSet, EcoQos, ResourcePriority};
use crate::model::{PriorityClass, ProcessKey};

/// 一个被 Poise 调整过的进程及其原始优先级。服务把它们存到磁盘上：服务异常退出（崩溃、被强行结束）
/// 来不及撤销时，下次启动按它把还在运行的进程恢复原样（见 [`restore_actions`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjusted {
    pub key: ProcessKey,
    pub original_priority: PriorityClass,
}

/// 上次运行留下、没能撤销的调整：不知道当时具体调成了什么，每一项都恢复一遍。
pub(super) fn restore_actions(leftover: &Adjusted) -> Vec<PlannedAction> {
    let pid = leftover.key.pid;
    [
        Action::SetPriority {
            pid,
            class: leftover.original_priority,
        },
        Action::SetEcoQos {
            pid,
            state: EcoQos::Default,
        },
        Action::SetCoreSet {
            pid,
            set: CoreSet::All,
        },
        Action::SetResourcePriority {
            pid,
            priority: ResourcePriority::Default,
        },
    ]
    .into_iter()
    .map(|action| PlannedAction {
        key: leftover.key,
        action,
        original_priority: leftover.original_priority,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionKind;

    #[test]
    fn leftovers_are_restored_in_every_respect_and_only_restored() {
        let leftover = Adjusted {
            key: ProcessKey { pid: 7, created: 1 },
            original_priority: PriorityClass::Normal,
        };
        let actions = restore_actions(&leftover);
        assert_eq!(actions.len(), 4);
        // 都是"恢复"：安全守卫总会放行，不会因为程序此刻在用而拦下。
        assert!(
            actions
                .iter()
                .all(|a| a.action.kind(a.original_priority) == ActionKind::Restore)
        );
    }
}
