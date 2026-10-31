//! 重点程序：不只是游戏——正被直接使用、持续需要处理器的任何程序，都按需要占用大核，
//! 其余没在用的程序让到小核（不是游戏的要持续一分钟），同一个程序固定在同一组小核里。

use super::*;
use crate::fast_hash::HashSet;

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

    // 四五个编译进程：6 个大核放得下。编辑器马上用上大核；短的编译不为它挪动别的程序。
    sim.proc_mut(BUILD).cpu_pct = 450.0;
    sim.run_for(40_000);
    assert_eq!(sim.cores_of(EDITOR), CoreSet::Domains(0b1));
    for idle in 10..20 {
        assert_eq!(sim.state_of(idle), None);
    }
    // 持续编译一分钟以上：闲着的程序让到小核。
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

/// 在经典的命令提示符里编译：窗口属于命令提示符（Windows 自带，不参与调度），编译器挂在它下面。
/// 整个程序照样是重点程序：编译器放到大核，闲着的程序让到小核；命令提示符本身不碰。
#[test]
fn a_build_in_a_system_console_is_focused_too() {
    const CONSOLE: Pid = 1;
    let console = Proc {
        system: true,
        ..app(CONSOLE)
    };
    let mut procs = vec![console, child(BUILD, CONSOLE)];
    procs.extend((10..13).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(CONSOLE);
    sim.proc_mut(BUILD).cpu_pct = 450.0;
    sim.run_for(2 * 60_000);
    assert_eq!(sim.cores_of(BUILD), CoreSet::Domains(0b1));
    assert!(sim.actions_on(CONSOLE).is_empty());
    // 日志和核心面板按树里参与调度的进程记：是重点程序，而不是"正常"。
    assert_eq!(sim.verdicts_of(CONSOLE), [Verdict::Focused]);
    for idle in 10..13 {
        assert_eq!(sim.state_of(idle), Some(Desired::ASIDE));
    }
}

/// 重点程序需要的线程超过大核（6 个物理核心，超线程挤在同一个核心上快不了多少）：先多用一组小核
/// （同组共用缓存），后台让路的程序搬到另一组，不和它挤。
#[test]
fn a_program_outgrowing_the_performance_cores_takes_one_cache_group() {
    const SYNC: Pid = 3;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 800.0,
            ..app(EDITOR)
        },
        Proc {
            cpu_pct: 60.0,
            ..app(SYNC)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(2 * 60_000);
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
    let mut used = HashSet::default();
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
    // 游戏需要的线程超过一个 CCD（连续两轮）：放开到所有核心。
    sim.proc_mut(GAME).cpu_pct = 1500.0;
    sim.run_for(2 * TICK_MS);
    assert_eq!(sim.cores_of(GAME), CoreSet::All);
}

/// 只有两个大核的低压处理器（i5-1235U：2 个大核 4 线程，两组各 4 个小核）：游戏不关在大核里——它的工作线程
/// 只能挤在大核的超线程上，比不管还慢；没在用的程序照样让到小核，大核空出来给游戏。
#[test]
fn a_game_is_not_confined_to_two_performance_cores() {
    const GAME: Pid = 1;
    const IDLE: Pid = 3;
    let cfg = ScheduleConfig {
        cores: CoreLayout::with_cores(&[(4, 2), (4, 4), (4, 4)]),
        ..Default::default()
    };
    let game = Proc {
        cpu_pct: 300.0,
        preference: AppPreference::Game,
        ..app(GAME)
    };
    let mut sim = Sim::with(cfg, vec![game, app(IDLE)]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert_eq!(sim.cores_of(GAME), CoreSet::All);
    assert!(sim.cores_of(IDLE).is_efficiency());
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
    // 没有挪核心，日志也不说"移到小核"。
    assert!(
        !sim.decisions
            .iter()
            .any(|d| matches!(d, Decision::Aside(n) if *n > 0))
    );
    assert!(!sim.verdicts_of(3).contains(&Verdict::Aside));
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

/// 日志复盘：浏览网页、打开聊天窗口，前台程序一阵一阵地忙（每次几秒）。不算重点程序，
/// 别的程序一个都不挪（以前十几个程序跟着挪到小核、二十秒后又挪回来）。
#[test]
fn replay_browsing_bursts_never_move_other_programs() {
    const BROWSER: Pid = 1;
    let mut procs = vec![app(BROWSER)];
    procs.extend((10..20).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(BROWSER);
    for _ in 0..9 {
        sim.proc_mut(BROWSER).cpu_pct = 150.0;
        sim.run_for(6_000);
        sim.proc_mut(BROWSER).cpu_pct = 10.0;
        sim.run_for(14_000);
    }
    assert!(!sim.verdicts_of(BROWSER).contains(&Verdict::Focused));
    assert!(
        !sim.decisions
            .iter()
            .any(|d| matches!(d, Decision::Aside(_)))
    );
    for idle in 10..20 {
        assert!(sim.actions_on(idle).is_empty(), "{idle}");
    }
}

/// 日志复盘：正在用、持续干重活的程序，前台被别的窗口抢来抢去（通知弹窗、切出去看一眼）。
/// 它一直是重点程序，核心范围只设一次，不来回切换（以前两分钟里来回四十多次）。
#[test]
fn replay_a_busy_program_keeps_its_cores_while_the_foreground_flickers() {
    const RENDER: Pid = 1;
    const CHAT: Pid = 2;
    let mut procs = vec![
        Proc {
            cpu_pct: 200.0,
            ..app(RENDER)
        },
        app(CHAT),
    ];
    procs.extend((10..15).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(RENDER);
    sim.run_for(30_000);
    assert_eq!(sim.verdicts_of(RENDER), [Verdict::Focused]);
    for i in 0..60 {
        sim.foreground = Some(if i % 2 == 0 { CHAT } else { RENDER });
        sim.run_for(TICK_MS);
    }
    assert_eq!(sim.verdicts_of(RENDER), [Verdict::Focused]);
    let core_changes = sim
        .actions_on(RENDER)
        .iter()
        .filter(|a| matches!(a, Action::SetCoreSet { .. }))
        .count();
    assert_eq!(core_changes, 1);
    // 真的离开了（切到别的程序干别的事）：过一会儿不再是重点程序，核心范围放开。
    sim.foreground = Some(CHAT);
    sim.run_for(60_000);
    assert_eq!(sim.cores_of(RENDER), CoreSet::All);
    assert_eq!(sim.verdicts_of(RENDER).last(), Some(&Verdict::Normal));
}

/// 日志复盘（0.7.0）：一个在发声、持续干重活的程序（本地语音识别之类），用户在它和别的窗口之间
/// 来回切换。以前每隔几秒就在"重点程序"和"不再优先用大核（在发声）"之间来回切换。
#[test]
fn replay_an_audible_busy_program_keeps_its_cores_while_the_user_switches_away_briefly() {
    const BUZZ: Pid = 1;
    const OTHER: Pid = 2;
    let mut procs = vec![
        Proc {
            cpu_pct: 230.0,
            disk_mb_s: 12.0,
            audio: true,
            ..app(BUZZ)
        },
        app(OTHER),
    ];
    procs.extend((10..15).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(BUZZ);
    sim.run_for(20_000);
    for _ in 0..8 {
        sim.foreground = Some(OTHER);
        sim.run_for(2 * TICK_MS);
        sim.foreground = Some(BUZZ);
        sim.run_for(6 * TICK_MS);
    }
    assert_eq!(sim.verdicts_of(BUZZ), [Verdict::Focused]);
}
