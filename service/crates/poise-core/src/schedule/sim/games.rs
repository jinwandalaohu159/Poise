//! 玩游戏：游戏的识别、模式切换与保持、核心分配，以及玩游戏时后台的让路。

use super::*;

/// 复盘"玩游戏时前台卡、声音卡"的场景：后台什么都没干，只有游戏、放着歌的浏览器和一堆托盘程序。
#[test]
fn replay_gaming_with_an_idle_background() {
    const GAME: Pid = 1;
    const BROWSER: Pid = 2;
    const RENDERER: Pid = 3;
    let mut procs = vec![
        Proc {
            cpu_pct: 350.0,
            renders_fullscreen: true,
            ..app(GAME)
        },
        app(BROWSER),
        Proc {
            cpu_pct: 8.0,
            ..child(RENDERER, BROWSER)
        },
        Proc {
            audio: true,
            ..child(4, BROWSER)
        },
    ];
    procs.extend((10..30).map(app));
    let mut sim = Sim::new(procs);

    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 玩到一半切出去看一眼浏览器，再切回来，来回几次。
    for _ in 0..6 {
        sim.foreground = Some(BROWSER);
        sim.run_for(10_000);
        sim.foreground = Some(GAME);
        sim.run_for(20_000);
    }
    sim.run_for(5 * 60_000);

    assert_eq!(sim.mode(), Mode::Full);
    // 游戏一开就放到大核，确认后再不让系统节能限速（两次）；浏览器里输出声音的进程不受限速；
    // 闲着的托盘程序只让到小核（大核留给游戏，不降优先级、不节能），各只调整一次，
    // 切来切去也不再动；浏览器的其他进程在发声，交给它自己和系统管理。
    let mut expected: HashMap<Pid, usize> = (10..30).map(|p| (p, 1)).collect();
    expected.extend([(GAME, 2), (4, 1)]);
    assert_eq!(sim.adjustments_per_process(), expected);
    for idle in 10..30 {
        assert!(
            sim.actions_on(idle)
                .iter()
                .all(|a| matches!(a, Action::SetCoreSet { set, .. } if set.is_efficiency()))
        );
        assert_eq!(sim.state_of(idle), Some(Desired::ASIDE));
    }
    // 托盘程序分摊在两组小核上，没有一组留给游戏（游戏只用大核就够）。
    let groups: HashSet<CoreSet> = (10..30).map(|p| sim.cores_of(p)).collect();
    assert_eq!(
        groups,
        HashSet::from([CoreSet::Domains(0b010), CoreSet::Domains(0b100)])
    );
    assert_eq!(sim.state_of(GAME), Some(Desired::PLAYING));
    assert!(sim.never_lowered(GAME));
    assert!(sim.only_protected(4) && sim.is_protected(4));
    // 一开始先按重点程序放到大核，确认后按游戏调度；切出去看一眼不算结束游戏，不再有别的记录。
    assert_eq!(sim.verdicts_of(GAME), [Verdict::Focused, Verdict::Playing]);
}

#[test]
fn games_switch_the_mode_and_stay_active_when_alt_tabbed() {
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            ..app(1)
        },
        app(2),
    ]);
    // 游戏启动：先在加载，过一会儿全屏开始渲染（显卡占用要测一次才知道，那时已经量到了它的处理器占用）。
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    sim.proc_mut(1).renders_fullscreen = true;
    sim.run_for(TICK_MS);
    // 自动识别的游戏要持续几秒才确认，但不等确认就先放到大核；别的程序不等确认不让路。
    assert_eq!(sim.mode(), Mode::Balanced);
    assert_eq!(sim.cores_of(1), CoreSet::Domains(0b1));
    assert_eq!(sim.state_of(2), None);
    sim.run_for(6_000);
    assert_eq!(sim.mode(), Mode::Full);
    assert_eq!(
        sim.last_plan.as_ref().unwrap().game.as_ref().map(|g| g.pid),
        Some(1)
    );
    sim.foreground = Some(2);
    sim.run_for(10 * 60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 游戏在大核上运行，不提优先级、不被降级；切到后台也保持，不来回调整。
    assert_eq!(sim.state_of(1), Some(Desired::PLAYING));
    assert!(sim.never_lowered(1));
    assert!(sim.actions_on(1).contains(&&Action::SetCoreSet {
        pid: 1,
        set: CoreSet::Domains(0b1)
    }));
    assert_eq!(sim.adjustments_per_process().get(&1), Some(&2));
}

/// 游戏挂在后台不玩（最小化、暂停，几乎不用处理器）：过一阵放到一边，回到日常模式，别的程序不再为它让路；
/// 切回游戏立即恢复，不用重新等确认。
#[test]
fn a_game_left_idle_in_the_background_is_set_aside_until_it_comes_back() {
    const GAME: Pid = 1;
    const BROWSER: Pid = 2;
    let game = Proc {
        cpu_pct: 200.0,
        preference: AppPreference::Game,
        ..app(GAME)
    };
    let mut sim = Sim::new(vec![game, app(BROWSER)]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 最小化，去浏览网页。
    sim.foreground = Some(BROWSER);
    sim.proc_mut(GAME).cpu_pct = 2.0;
    sim.run_for(2 * 60_000);
    assert_eq!(sim.mode(), Mode::Full);
    sim.run_for(5 * 60_000);
    assert_eq!(sim.mode(), Mode::Balanced);
    assert!(sim.last_plan.as_ref().unwrap().game.is_none());
    // 切回游戏：立即恢复。
    sim.foreground = Some(GAME);
    sim.proc_mut(GAME).cpu_pct = 200.0;
    sim.run_for(TICK_MS);
    assert_eq!(sim.mode(), Mode::Full);
}

/// 复盘"一局结束回到客户端，选完英雄再进下一局"：中间不切回日常模式（每次切换散热模式都卡一下）。
#[test]
fn game_mode_is_kept_between_matches() {
    const CLIENT: Pid = 1;
    const MATCH: Pid = 2;
    let game = Proc {
        cpu_pct: 150.0,
        preference: AppPreference::Game,
        ..app(MATCH)
    };
    let mut sim = Sim::new(vec![app(CLIENT), game.clone()]);
    sim.foreground = Some(MATCH);
    sim.run_for(20 * 60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 一局结束：游戏进程退出，回到客户端选英雄。
    sim.procs.retain(|p| p.pid != MATCH);
    sim.foreground = Some(CLIENT);
    sim.run_for(90_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 下一局开始。
    sim.procs.push(game.clone());
    sim.foreground = Some(MATCH);
    sim.run_for(60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 不玩了：会话结束后回到日常模式。
    sim.procs.retain(|p| p.pid != MATCH);
    sim.foreground = Some(CLIENT);
    sim.run_for(4 * 60_000);
    assert_eq!(sim.mode(), Mode::Balanced);
}

/// 复盘"重度多线程的游戏"：需要的线程超过大核能提供的，不固定在大核，小核也能分担；
/// 只靠一两个主线程的游戏固定在大核。
#[test]
fn heavily_threaded_games_may_use_every_core() {
    let cfg = ScheduleConfig::default();
    assert_eq!(cfg.cores.threads(0), 12);
    let light = Proc {
        cpu_pct: 150.0,
        preference: AppPreference::Game,
        ..app(1)
    };
    let heavy = Proc {
        cpu_pct: 1400.0,
        preference: AppPreference::Game,
        ..app(2)
    };
    let mut sim = Sim::with(cfg, vec![light, heavy]);
    sim.foreground = Some(2);
    sim.run_for(3 * 60_000);
    assert_eq!(sim.state_of(1).map(|d| d.cores), Some(CoreSet::Performance));
    assert_eq!(sim.state_of(2).map(|d| d.cores), Some(CoreSet::All));
}

/// 复盘"玩游戏时，刚用过的客户端还在后台忙"：玩游戏时它更早让路，大核和功耗留给游戏；
/// 不玩游戏时，同样的占用不让路。
#[test]
fn background_work_yields_sooner_while_gaming() {
    const CLIENT: Pid = 1;
    const GAME: Pid = 2;
    let client = Proc {
        cpu_pct: 5.0,
        ..app(CLIENT)
    };
    let mut sim = Sim::new(vec![client.clone()]);
    sim.foreground = Some(CLIENT);
    sim.run_for(TICK_MS);
    // 不玩游戏：刚用过、占用不高，不让路。
    sim.foreground = None;
    sim.run_for(60_000);
    assert!(!sim.is_yielded(CLIENT));

    sim.procs.push(Proc {
        cpu_pct: 150.0,
        preference: AppPreference::Game,
        ..app(GAME)
    });
    sim.foreground = Some(GAME);
    // 刚用过的程序先缓一会儿（来回切换不反复调整），之后依然在忙才让路。
    sim.run_for(8_000);
    assert!(!sim.is_yielded(CLIENT));
    sim.run_for(16_000);
    assert!(sim.is_yielded(CLIENT));
    // 负载轻（5%，达不到平时的门槛）：挪开、降优先级，但不节能（不让它的输入、画面变迟）。
    assert_eq!(sim.state_of(CLIENT), Some(Desired::NUDGED));
    assert_eq!(sim.state_of(GAME), Some(Desired::GAME));
    // 回到客户端：立即恢复。
    sim.foreground = Some(CLIENT);
    sim.run_for(TICK_MS);
    assert!(!sim.is_yielded(CLIENT));
}

/// 游戏停在菜单里、人走开了：后台的下载不再按"需求高"的低门槛让路（只是轻负载的恢复，
/// 游戏自己照样在大核上）；人回来一动鼠标，立即回到需求高。
#[test]
fn walking_away_from_a_game_menu_lets_light_background_work_continue() {
    const GAME: Pid = 1;
    const DOWNLOAD: Pid = 2;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 30.0,
            preference: AppPreference::Game,
            ..app(GAME)
        },
        Proc {
            cpu_pct: 5.0,
            ..app(DOWNLOAD)
        },
    ]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert!(sim.is_yielded(DOWNLOAD));
    sim.idle_ms = 6 * 60_000;
    sim.run_for(60_000);
    assert!(!sim.is_yielded(DOWNLOAD));
    assert_eq!(sim.state_of(GAME), Some(Desired::GAME));
    sim.idle_ms = 0;
    sim.run_for(60_000);
    assert!(sim.is_yielded(DOWNLOAD));
}

/// 复盘"玩游戏时，聊天软件的通知弹窗一下下抢走前台"：每次被抢走都会立即恢复它（可能是用户切过去了），
/// 但随后不会马上又让路——不来回调整。
#[test]
fn foreground_flickers_while_gaming_do_not_flip_programs() {
    const GAME: Pid = 1;
    const CHAT: Pid = 2;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            preference: AppPreference::Game,
            ..app(GAME)
        },
        Proc {
            cpu_pct: 7.0,
            ..app(CHAT)
        },
    ]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert!(sim.is_yielded(CHAT));
    // 每 6 秒被抢一次前台，持续 3 分钟。
    for _ in 0..30 {
        sim.foreground = Some(CHAT);
        sim.run_for(TICK_MS);
        sim.foreground = Some(GAME);
        sim.run_for(4_000);
    }
    // 游戏一开先让到小核，在忙再让路（同一组小核，不再挪动）；第一次被抢时恢复，之后一直保持
    // （以前每次被抢都要恢复、再让路，3 分钟里来回 60 次）。
    assert_eq!(sim.adjustments_per_process().get(&CHAT), Some(&3));
    // 不再被打扰 20 秒后，依然在忙，再让路。
    sim.run_for(30_000);
    assert!(sim.is_yielded(CHAT));
    assert_eq!(sim.adjustments_per_process().get(&CHAT), Some(&4));
    use Verdict::{Normal, Yielded};
    assert_eq!(sim.verdicts_of(CHAT), [Yielded, Normal, Yielded]);
}

/// 不玩了：游戏会话结束后（最后一局结束一段时间，见 `game_session_hold_ms`），玩游戏时按更低的门槛
/// 让路的轻负载程序恢复；依然很忙的继续让路，但只降优先级（没有重点程序了，空闲的核心照样给它用）。
/// 会话结束之前（两局之间）照样让路，不来回切换。
#[test]
fn light_programs_come_back_after_the_game() {
    const GAME: Pid = 1;
    const LIGHT: Pid = 2;
    const HEAVY: Pid = 3;
    const USER_APP: Pid = 4;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            preference: AppPreference::Game,
            ..app(GAME)
        },
        Proc {
            cpu_pct: 5.0,
            ..app(LIGHT)
        },
        Proc {
            cpu_pct: 60.0,
            ..app(HEAVY)
        },
        Proc {
            cpu_pct: 50.0,
            ..app(USER_APP)
        },
    ]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert!(sim.is_yielded(LIGHT) && sim.is_yielded(HEAVY));
    // 游戏退出，用户回到自己的程序继续忙。
    sim.procs.retain(|p| p.pid != GAME);
    sim.foreground = Some(USER_APP);
    sim.run_for(90_000);
    assert!(sim.is_yielded(LIGHT) && sim.is_yielded(HEAVY));
    sim.run_for(3 * 60_000);
    assert_eq!(sim.state_of(LIGHT), None);
    assert!(sim.is_lowered(HEAVY));
}

/// 复盘一整段英雄联盟：客户端里排队 → 进游戏 → 中途切出去看两次浏览器 → 一局结束回客户端选英雄 →
/// 再来一局 → 不玩了。开着语音、浏览器、一个在后台更新的程序和一堆托盘程序。
/// 托盘程序在整段时间里只挪动一次（开局让到小核）、最后恢复一次；两局之间不来回挪。
#[test]
fn replay_two_matches_with_a_lobby_in_between() {
    const CLIENT: Pid = 1;
    const GAME: Pid = 2;
    const VOICE: Pid = 3;
    const BROWSER: Pid = 4;
    const UPDATER: Pid = 5;
    let game = Proc {
        cpu_pct: 250.0,
        renders_fullscreen: true,
        ..app(GAME)
    };
    let mut procs = vec![
        Proc {
            cpu_pct: 40.0,
            ..app(CLIENT)
        },
        Proc {
            cpu_pct: 3.0,
            audio: true,
            ..app(VOICE)
        },
        app(BROWSER),
        Proc {
            cpu_pct: 30.0,
            ..app(UPDATER)
        },
    ];
    procs.extend((5..10).map(|i| child(BROWSER * 10 + i, BROWSER)));
    procs.extend((100..110).map(app));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(CLIENT);
    sim.run_for(60_000);
    for round in 0..2 {
        // 进游戏：客户端闲下来。
        sim.procs.push(game.clone());
        sim.proc_mut(CLIENT).cpu_pct = 5.0;
        sim.foreground = Some(GAME);
        sim.run_for(5 * 60_000);
        for _ in 0..2 {
            sim.foreground = Some(BROWSER);
            sim.run_for(15_000);
            sim.foreground = Some(GAME);
            sim.run_for(3 * 60_000);
        }
        // 一局结束，回客户端；第一局之后选英雄 90 秒。
        sim.procs.retain(|p| p.pid != GAME);
        sim.proc_mut(CLIENT).cpu_pct = 40.0;
        sim.foreground = Some(CLIENT);
        sim.run_for(if round == 0 { 90_000 } else { 5 * 60_000 });
    }
    // 两局之间（选英雄）后台照样完整让路，不在"只降优先级"和"完整让路"之间来回切换。
    use Verdict::{Lowered, Yielded};
    assert_eq!(sim.verdicts_of(UPDATER), [Lowered, Yielded, Lowered]);
    for tray in 100..110 {
        assert_eq!(sim.adjustments_per_process()[&tray], 2, "托盘程序 {tray}");
        assert_eq!(sim.state_of(tray), None);
    }
    // 语音从头到尾只被保护一次。
    assert!(sim.only_protected(VOICE));
    assert_eq!(sim.adjustments_per_process()[&VOICE], 1);
    // 不玩了：模式回到日常，更新程序最多只降优先级（用户在客户端里忙）。
    assert_eq!(sim.mode(), Mode::Balanced);
    assert!(sim.state_of(UPDATER).is_none_or(|d| d == Desired::LOWERED));
}

/// 日志复盘：像游戏（前台全屏、显卡在渲染）的进程却一点处理器都不用（例如游戏的崩溃处理进程被当成了
/// 前台）：不当成游戏放到大核，也不为它让路。
#[test]
fn a_lookalike_game_doing_no_work_gets_nothing() {
    const HELPER: Pid = 1;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 0.0,
            ..app(HELPER)
        },
        app(2),
    ]);
    sim.foreground = Some(HELPER);
    sim.run_for(TICK_MS);
    sim.proc_mut(HELPER).renders_fullscreen = true;
    sim.run_for(4_000);
    assert_eq!(sim.cores_of(HELPER), CoreSet::All);
    assert!(!sim.verdicts_of(HELPER).contains(&Verdict::Focused));
    assert!(sim.actions_on(2).is_empty());
}
