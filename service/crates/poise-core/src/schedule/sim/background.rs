//! 后台让路：什么时候让、让给谁、按程序（进程树）判断，以及不来回调整。

use super::*;

/// 复盘"不玩游戏，在几个程序之间来回切换"：调度不应该跟着来回折腾。
#[test]
fn replay_switching_between_busy_programs() {
    // 三个程序都在干活（编辑器、浏览器、聊天软件），用户在它们之间来回切换。
    let mut sim = Sim::new(
        [1, 2, 3]
            .map(|pid| Proc {
                cpu_pct: 30.0,
                ..app(pid)
            })
            .to_vec(),
    );
    // 程序是用户打开的：打开时都在前台待过。
    for pid in [1, 2, 3] {
        sim.foreground = Some(pid);
        sim.run_for(TICK_MS);
    }
    for round in 0..60 {
        sim.foreground = Some([1, 2, 3][round % 3]);
        sim.run_for(10_000);
    }
    // 最近用过的程序都算"正在用"，切来切去一次都没被降级。
    assert!(sim.log.is_empty(), "{:?}", sim.log);
}

/// 复盘"后台有个程序在持续干活"：
/// - 用户没在用电脑（前台闲着）：没人需要处理器，它全速干活，不让路；
/// - 用户开始干活：它让路一次，之后时忙时闲也一直保持，不来回调整；
/// - 用户点回它：立即恢复；用户离开一会儿（持续没有需求）：恢复全速，让后台的工作尽快完成。
#[test]
fn replay_background_work_yields_only_when_someone_needs_the_processor() {
    const USER_APP: Pid = 1;
    const UPDATER: Pid = 40;
    let mut procs = vec![app(USER_APP)];
    procs.extend((10..20).map(app));
    procs.push(Proc {
        cpu_pct: 60.0,
        ..app(UPDATER)
    });
    let mut sim = Sim::new(procs);
    sim.foreground = Some(USER_APP);
    sim.run_for(2 * 60_000);
    assert!(sim.log.is_empty(), "{:?}", sim.log);

    // 用户开始干活：后台的它让路。
    sim.proc_mut(USER_APP).cpu_pct = 80.0;
    sim.run_for(30_000);
    assert!(sim.is_yielded(UPDATER));
    // 时忙时闲也不来回调整。
    for _ in 0..10 {
        sim.proc_mut(UPDATER).cpu_pct = 0.0;
        sim.run_for(10_000);
        sim.proc_mut(UPDATER).cpu_pct = 60.0;
        sim.run_for(10_000);
    }
    assert!(sim.is_yielded(UPDATER));
    assert_eq!(sim.adjustments_per_process(), HashMap::from([(UPDATER, 1)]));

    // 用户离开电脑：持续没有需求后，它恢复全速。
    sim.proc_mut(USER_APP).cpu_pct = 0.0;
    sim.run_for(2 * 60_000);
    assert!(!sim.is_yielded(UPDATER));
    // 用户回来干活：它再让路。
    sim.proc_mut(USER_APP).cpu_pct = 80.0;
    sim.run_for(30_000);
    assert!(sim.is_yielded(UPDATER));
    // 用户点开它：立即恢复。
    sim.foreground = Some(UPDATER);
    sim.run_for(TICK_MS);
    assert!(!sim.is_yielded(UPDATER));

    assert!(sim.never_lowered(USER_APP));
    for pid in 10..20 {
        assert!(sim.actions_on(pid).is_empty());
    }
    // 日志里每次让路、恢复各一条，用户的程序和闲着的程序一条都没有。
    use Verdict::{Normal, Yielded};
    assert_eq!(sim.verdicts_of(UPDATER), [Yielded, Normal, Yielded, Normal]);
    assert!((1..20).all(|pid| sim.verdicts_of(pid).is_empty()));
}

/// 复盘"后台编译"：编译器不断新建、几秒就退出的子进程，单看哪个进程都不忙，整个程序却很忙。
/// 按程序（进程树）判断：整棵树让路，之后新建的子进程也跟着让路。
#[test]
fn programs_are_judged_as_whole_process_trees() {
    const USER_APP: Pid = 1;
    const BUILD: Pid = 50;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 80.0,
            ..app(USER_APP)
        },
        app(BUILD),
    ]);
    sim.foreground = Some(USER_APP);
    let mut next_pid = 100;
    for _ in 0..30 {
        // 每 4 秒换一批编译进程，每个只活 4 秒。
        sim.procs.retain(|p| p.parent != Some(BUILD));
        for _ in 0..3 {
            sim.procs.push(Proc {
                cpu_pct: 90.0,
                ..child(next_pid, BUILD)
            });
            next_pid += 1;
        }
        sim.run_for(4_000);
    }
    assert!(sim.is_yielded(BUILD));
    let children: Vec<Pid> = sim
        .procs
        .iter()
        .filter(|p| p.parent == Some(BUILD))
        .map(|p| p.pid)
        .collect();
    assert!(children.iter().all(|&pid| sim.is_yielded(pid)));
}

/// 用户标记的游戏、始终优先的程序：在后台也不让系统限速（窗口化、没被自动识别的游戏靠标记）；
/// 标记的游戏另外提高一级优先级。
#[test]
fn marked_programs_are_protected_in_the_background() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            cpu_pct: 200.0,
            preference: AppPreference::Game,
            ..app(2)
        },
        Proc {
            cpu_pct: 50.0,
            preference: AppPreference::AlwaysFast,
            ..app(3)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(5 * 60_000);
    assert_eq!(sim.mode(), Mode::Full);
    assert!(sim.is_protected(3) && sim.only_protected(3));
    assert!(matches!(
        sim.scheduler.state(&ProcessKey { pid: 2, created: 1 }),
        ProcessSchedule::Adjusted(d) if d == Desired::GAME
    ));
    assert!(sim.actions_on(2).contains(&&Action::SetPriority {
        pid: 2,
        class: PriorityClass::AboveNormal
    }));
    assert_eq!(
        sim.adjustments_per_process(),
        HashMap::from([(2, 1), (3, 1)])
    );
    assert!(sim.actions_on(1).is_empty());
}

#[test]
fn low_priority_programs_stay_on_small_cores() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert!(sim.is_yielded(2));
    sim.foreground = Some(2);
    sim.run_for(TICK_MS);
    assert!(matches!(
        sim.scheduler.state(&ProcessKey { pid: 2, created: 1 }),
        ProcessSchedule::Adjusted(d) if d == Desired::LOW_IN_FOREGROUND
    ));
}

/// 复盘"用电池时后台在忙"：即使前台闲着，后台忙碌的程序也让路（在小核上节能运行，省电）；
/// 接上电源、又没人需要处理器时恢复。
#[test]
fn on_battery_busy_background_work_yields_to_save_power() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            cpu_pct: 40.0,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(60_000);
    assert!(!sim.is_yielded(2));
    sim.on_battery = true;
    sim.run_for(30_000);
    assert!(sim.is_yielded(2));
    sim.on_battery = false;
    sim.run_for(2 * 60_000);
    assert!(!sim.is_yielded(2));
}

#[test]
fn non_hybrid_cpus_never_restrict_cores() {
    let mut sim = Sim::with(
        ScheduleConfig {
            cores: CoreLayout::uniform(16),
            ..Default::default()
        },
        vec![
            app(1),
            Proc {
                preference: AppPreference::Background,
                ..app(2)
            },
        ],
    );
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert!(!sim.actions_on(2).is_empty());
    assert!(
        !sim.log
            .iter()
            .any(|(_, a)| matches!(a.action, Action::SetCoreSet { .. }))
    );
}

/// 小核太少的处理器：后台只用优先级和节能让路，不挤到一两个小核上；游戏照样固定在大核。
#[test]
fn too_few_small_cores_are_not_used_for_yielding() {
    let cfg = ScheduleConfig {
        cores: CoreLayout {
            performance_threads: 8,
            efficiency_threads: 2,
        },
        ..Default::default()
    };
    let mut sim = Sim::with(
        cfg,
        vec![
            app(1),
            Proc {
                preference: AppPreference::Background,
                ..app(2)
            },
            Proc {
                cpu_pct: 150.0,
                preference: AppPreference::Game,
                ..app(3)
            },
        ],
    );
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    let low = sim.state_of(2).unwrap();
    assert_eq!(low.cores, CoreSet::All);
    assert_eq!(low.priority, Some(PriorityClass::BelowNormal));
    assert_eq!(sim.state_of(3).map(|d| d.cores), Some(CoreSet::Performance));
}

/// 处理器占用很低、却一直大量读写磁盘的后台程序（同步、下载、备份）：有人需要时同样让路，
/// 内存与磁盘读写优先级一起降低；没人需要时不碰。
#[test]
fn heavy_disk_users_yield_when_someone_needs_the_machine() {
    const USER_APP: Pid = 1;
    const SYNC: Pid = 2;
    let mut sim = Sim::new(vec![
        app(USER_APP),
        Proc {
            cpu_pct: 1.0,
            disk_mb_s: 40.0,
            ..app(SYNC)
        },
    ]);
    sim.foreground = Some(USER_APP);
    sim.run_for(2 * 60_000);
    assert!(!sim.is_yielded(SYNC));
    sim.proc_mut(USER_APP).cpu_pct = 80.0;
    sim.run_for(60_000);
    assert!(sim.is_yielded(SYNC));
    assert_eq!(
        sim.state_of(SYNC).map(|d| d.resources),
        Some(ResourcePriority::Low)
    );
}
