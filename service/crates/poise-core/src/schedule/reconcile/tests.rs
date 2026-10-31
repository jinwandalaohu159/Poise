use super::*;
use crate::action::{CoreSet, EcoQos, ResourcePriority};

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
        eco: EcoQos::On,
        ..LOW
    };
    let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
    // 优先级、节能、核心范围、内存优先级。
    assert_eq!(actions.len(), 4);
    apply_all(&mut r, &actions);
    assert!(
        r.diff(KEY, Some(PriorityClass::BelowNormal), false, want)
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
        resources: ResourcePriority::Low,
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
fn raising_stops_at_above_normal_and_never_lowers_a_higher_original() {
    let raise = |to| Desired {
        priority: Some(to),
        ..Desired::NEUTRAL
    };
    let mut r = Reconciler::default();
    let a = r.diff(
        KEY,
        Some(PriorityClass::Normal),
        false,
        raise(PriorityClass::AboveNormal),
    );
    assert_eq!(
        a[0].action,
        Action::SetPriority {
            pid: KEY.pid,
            class: PriorityClass::AboveNormal
        }
    );
    let mut r = Reconciler::default();
    assert!(
        r.diff(
            KEY,
            Some(PriorityClass::Normal),
            false,
            raise(PriorityClass::High)
        )
        .is_empty()
    );
    let mut r = Reconciler::default();
    assert!(
        r.diff(
            KEY,
            Some(PriorityClass::High),
            false,
            raise(PriorityClass::AboveNormal)
        )
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
fn failed_actions_wait_longer_each_time_before_being_retried() {
    let mut r = Reconciler::default();
    let want = Desired {
        eco: EcoQos::Off,
        ..Desired::NEUTRAL
    };
    let failed = Err(ExecError::Failed("x".into()));
    let mut at = 0;
    r.begin(at);
    let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
    r.report(&actions[0], &failed);
    for wait in [retry::FIRST_MS, retry::FIRST_MS * 2] {
        r.begin(at + wait - 1);
        assert!(
            r.diff(KEY, Some(PriorityClass::Normal), false, want)
                .is_empty()
        );
        at += wait;
        r.begin(at);
        let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
        assert_eq!(actions.len(), 1);
        r.report(&actions[0], &failed);
    }
    // 其他种类的动作不受影响；成功后不再等待。
    let lower = Desired {
        eco: EcoQos::Off,
        ..LOW
    };
    assert!(
        !r.diff(KEY, Some(PriorityClass::Normal), false, lower)
            .is_empty()
    );
    r.begin(at + retry::MAX_MS);
    let actions = r.diff(KEY, Some(PriorityClass::Normal), false, want);
    apply_all(&mut r, &actions);
    assert!(r.retries.is_empty());
}

#[test]
fn restore_all_undoes_everything() {
    let mut r = Reconciler::default();
    let want = Desired {
        priority: Some(PriorityClass::High),
        eco: EcoQos::Off,
        cores: CoreSet::All,
        resources: ResourcePriority::Low,
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
    resources: ResourcePriority::Low,
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
    // 退让：不碰优先级，只把节能、核心范围、内存优先级恢复默认。
    assert!(
        !last
            .iter()
            .any(|a| matches!(a.action, Action::SetPriority { .. }))
    );
    assert_eq!(last.len(), 3);
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

#[test]
fn yielding_never_raises_a_program_that_was_already_lower() {
    let mut r = Reconciler::default();
    let actions = r.diff(KEY, Some(PriorityClass::Idle), false, LOW);
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a.action, Action::SetPriority { .. })),
        "{actions:?}"
    );
    apply_all(&mut r, &actions);
    assert!(
        r.restore_all()
            .iter()
            .all(|a| !matches!(a.action, Action::SetPriority { .. }))
    );
}
