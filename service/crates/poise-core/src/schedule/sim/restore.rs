//! 还原：关闭智能调度、退出时撤销全部调整，以及继承了低优先级的子进程。

use super::*;

#[test]
fn turning_smart_scheduling_off_restores_everything() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert_eq!(sim.scheduler.adjusted_count(), 1);
    sim.tick_with(false);
    assert_eq!(sim.scheduler.adjusted_count(), 0);
}

#[test]
fn release_all_restores_every_adjusted_process() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    let restore = sim.scheduler.release_all();
    assert!(!restore.is_empty());
    assert!(restore.iter().all(|a| matches!(
        a.action,
        Action::SetPriority {
            class: PriorityClass::Normal,
            ..
        } | Action::SetEcoQos {
            state: EcoQos::Default,
            ..
        } | Action::SetCoreSet {
            set: CoreSet::All,
            ..
        } | Action::SetResourcePriority {
            priority: ResourcePriority::Default,
            ..
        }
    )));
}

#[test]
fn children_that_inherited_low_priority_are_restored_to_normal() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    // 父进程已被降低；之后新建的子进程继承了"低于正常"（Windows 的规则）。
    sim.procs.push(child(3, 2));
    let mut snap = sim.snapshot();
    for p in &mut snap.processes {
        if p.facts.key.pid == 3 {
            p.facts.priority = Some(PriorityClass::BelowNormal);
            p.facts.key.created = 2;
        }
    }
    sim.now += TICK_MS;
    snap.at = sim.now;
    let classes = sim.classes();
    let plan = sim.scheduler.plan(&ScheduleInput {
        snapshot: &snap,
        classes: &classes,
        activity: &sim.activity,
        enabled: true,
        user_mode: Mode::Balanced,
        game_mode: None,
        on_battery: false,
        hot: false,
    });
    for a in &plan.actions {
        sim.scheduler.report(a, &Ok(ExecOutcome::default()));
    }
    // 子进程自己不需要让路：立即恢复为正常，不会一直低着。
    assert!(plan.actions.iter().any(|a| a.key.pid == 3
        && a.action
            == Action::SetPriority {
                pid: 3,
                class: PriorityClass::Normal
            }));
}
