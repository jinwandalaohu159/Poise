//! 重点程序：不只是游戏——正被直接使用、持续需要处理器的任何程序，都按需要占用大核，
//! 其余没在用的程序让到小核，同一个程序固定在同一组小核里。

use std::collections::HashSet;

use super::*;

const EDITOR: Pid = 1;
const BUILD: Pid = 2;

/// 前台的编辑器里开始编译（编译器进程挂在编辑器名下）：编辑器整个程序放到大核，闲着的程序让到小核；
/// 编译结束、闲下来一会儿后，全部恢复。每个程序只挪动一次，结束时再挪回一次。
#[test]
fn a_busy_foreground_program_gets_the_performance_cores() {
    let mut procs = vec![app(EDITOR), child(BUILD, EDITOR)];
    procs.extend((10..20).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(EDITOR);
    // 只是打字、浏览：负载不高，谁都不动。
    sim.run_for(60_000);
    assert!(sim.log.is_empty());

    sim.proc_mut(BUILD).cpu_pct = 600.0;
    sim.run_for(60_000);
    assert_eq!(sim.cores_of(EDITOR), CoreSet::Domains(0b1));
    assert_eq!(sim.cores_of(BUILD), CoreSet::Domains(0b1));
    assert!(sim.verdicts_of(EDITOR).contains(&Verdict::Focused));
    for idle in 10..20 {
        assert_eq!(sim.state_of(idle), Some(Desired::ASIDE));
    }
    assert!(sim.decisions.contains(&Decision::Aside(10)));
    assert!(sim.last_plan.as_ref().unwrap().focused);

    // 编译结束。
    sim.proc_mut(BUILD).cpu_pct = 0.0;
    sim.run_for(3 * 60_000);
    assert_eq!(sim.cores_of(EDITOR), CoreSet::All);
    assert!(!sim.last_plan.as_ref().unwrap().focused);
    for idle in 10..20 {
        assert_eq!(sim.state_of(idle), None);
        assert_eq!(sim.adjustments_per_process()[&idle], 2);
        assert!(sim.never_lowered_priority(idle));
    }
    let ended = sim.decisions.iter().position(|d| *d == Decision::Aside(0));
    let started = sim.decisions.iter().position(|d| *d == Decision::Aside(10));
    assert!(started < ended && ended.is_some());
}

/// 重点程序需要的线程超过大核：先多用一组小核（同组共用缓存），后台让路的程序搬到另一组，不和它挤。
#[test]
fn a_program_outgrowing_the_performance_cores_takes_one_cache_group() {
    const SYNC: Pid = 3;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 1000.0,
            ..app(EDITOR)
        },
        Proc {
            cpu_pct: 60.0,
            ..app(SYNC)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(60_000);
    assert_eq!(sim.cores_of(EDITOR), CoreSet::Domains(0b011));
    assert!(sim.is_yielded(SYNC));
    assert_eq!(sim.cores_of(SYNC), CoreSet::Domains(0b100));
}

/// 让到小核的程序：同一个程序的所有进程在同一组小核里，不同的程序分摊到各组。
#[test]
fn every_process_of_a_program_shares_one_cache_group() {
    const GAME: Pid = 1;
    let mut procs = vec![Proc {
        cpu_pct: 200.0,
        preference: AppPreference::Game,
        ..app(GAME)
    }];
    for root in [10, 20, 30, 40] {
        procs.push(app(root));
        procs.extend((1..4).map(|i| child(root + i, root)));
    }
    let mut sim = Sim::new(procs);
    sim.foreground = Some(GAME);
    sim.run_for(30_000);
    let mut used = HashSet::new();
    for root in [10, 20, 30, 40] {
        let group = sim.cores_of(root);
        assert!(group.is_efficiency());
        for i in 1..4 {
            assert_eq!(sim.cores_of(root + i), group);
        }
        used.insert(group);
    }
    assert_eq!(used.len(), 2);
}

/// 双 CCD 的锐龙（每个 CCD 一块三级缓存，没有大小核之分）：同一套规则——游戏放在一个 CCD 上
/// （线程之间不跨 CCD 通信），没在用的、后台忙的程序放到另一个 CCD；游戏需要更多线程时再放开。
#[test]
fn dual_ccd_processors_follow_the_same_rules() {
    const GAME: Pid = 1;
    const UPDATER: Pid = 2;
    const IDLE: Pid = 3;
    let cfg = ScheduleConfig {
        cores: CoreLayout::new(&[16, 16]),
        ..Default::default()
    };
    let mut sim = Sim::with(
        cfg,
        vec![
            Proc {
                cpu_pct: 400.0,
                preference: AppPreference::Game,
                ..app(GAME)
            },
            Proc {
                cpu_pct: 50.0,
                ..app(UPDATER)
            },
            app(IDLE),
        ],
    );
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert_eq!(sim.cores_of(GAME), CoreSet::Domains(0b01));
    assert_eq!(sim.cores_of(UPDATER), CoreSet::Domains(0b10));
    assert!(sim.is_yielded(UPDATER));
    assert_eq!(sim.state_of(IDLE), Some(Desired::ASIDE));
    // 游戏需要的线程超过一个 CCD：放开到所有核心。
    sim.proc_mut(GAME).cpu_pct = 1500.0;
    sim.run_for(TICK_MS);
    assert_eq!(sim.cores_of(GAME), CoreSet::All);
}

/// 只有一块缓存、没有大小核的处理器：不限制任何核心，只用优先级和节能区分。
#[test]
fn single_domain_processors_never_restrict_cores() {
    let cfg = ScheduleConfig {
        cores: CoreLayout::uniform(16),
        ..Default::default()
    };
    let mut sim = Sim::with(
        cfg,
        vec![
            Proc {
                cpu_pct: 400.0,
                preference: AppPreference::Game,
                ..app(1)
            },
            Proc {
                cpu_pct: 50.0,
                ..app(2)
            },
            app(3),
        ],
    );
    sim.foreground = Some(1);
    sim.run_for(60_000);
    assert!(
        !sim.log
            .iter()
            .any(|(_, a)| matches!(a.action, Action::SetCoreSet { .. }))
    );
    assert!(sim.actions_on(3).is_empty());
    assert_eq!(
        sim.state_of(2).map(|d| d.priority),
        Some(Some(PriorityClass::BelowNormal))
    );
}

/// 用户设为"低优先级"的程序在前台很忙：照用户的留在小核，也不让别的程序为它让路。
#[test]
fn a_program_marked_low_never_becomes_the_focus() {
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 600.0,
            preference: AppPreference::Background,
            ..app(1)
        },
        app(2),
    ]);
    sim.foreground = Some(1);
    sim.run_for(60_000);
    assert_eq!(sim.state_of(1), Some(Desired::LOW_IN_FOREGROUND));
    assert!(sim.actions_on(2).is_empty());
}
