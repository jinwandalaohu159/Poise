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
    // 后台什么都没干：闲着的程序一个都没碰。游戏放到大核、浏览器里输出声音的进程不受限速，
    // 各只调整一次，切来切去也不再动；浏览器的其他进程交给它自己和系统管理。
    let expected: HashMap<Pid, usize> = [GAME, 4].map(|p| (p, 1)).into();
    assert_eq!(sim.adjustments_per_process(), expected);
    assert_eq!(sim.state_of(GAME), Some(Desired::PLAYING));
    assert!(sim.never_lowered(GAME));
    assert!(sim.only_protected(4) && sim.is_protected(4));
    // 切出去看一眼不算结束游戏：日志里只有一条"按游戏调度"。
    assert_eq!(sim.verdicts_of(GAME), [Verdict::Playing]);
}

#[test]
fn games_switch_the_mode_and_stay_active_when_alt_tabbed() {
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            renders_fullscreen: true,
            ..app(1)
        },
        app(2),
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    // 自动识别的游戏要持续几秒才确认。
    assert_eq!(sim.mode(), Mode::Balanced);
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
        set: CoreSet::Performance
    }));
    assert_eq!(sim.adjustments_per_process().get(&1), Some(&1));
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
    assert_eq!(cfg.cores.performance_threads, 12);
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
    assert_eq!(sim.state_of(GAME), Some(Desired::GAME));
    // 回到客户端：立即恢复。
    sim.foreground = Some(CLIENT);
    sim.run_for(TICK_MS);
    assert!(!sim.is_yielded(CLIENT));
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
    // 第一次被抢时恢复，之后一直保持（以前每次被抢都要恢复、再让路，3 分钟里来回 60 次）。
    assert_eq!(sim.adjustments_per_process().get(&CHAT), Some(&2));
    // 不再被打扰 20 秒后，依然在忙，再让路。
    sim.run_for(30_000);
    assert!(sim.is_yielded(CHAT));
    assert_eq!(sim.adjustments_per_process().get(&CHAT), Some(&3));
    use Verdict::{Normal, Yielded};
    assert_eq!(sim.verdicts_of(CHAT), [Yielded, Normal, Yielded]);
}

/// 一局结束：玩游戏时按更低的门槛让路的轻负载程序恢复，依然很忙的继续让路。
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
            cpu_pct: 80.0,
            ..app(USER_APP)
        },
    ]);
    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert!(sim.is_yielded(LIGHT) && sim.is_yielded(HEAVY));
    // 游戏退出，用户回到自己的程序继续忙。
    sim.procs.retain(|p| p.pid != GAME);
    sim.foreground = Some(USER_APP);
    sim.run_for(60_000);
    assert!(!sim.is_yielded(LIGHT));
    assert!(sim.is_yielded(HEAVY));
}
