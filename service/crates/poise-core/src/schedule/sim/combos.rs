//! 几种程序一起用（游戏、编译、容器、浏览器、通话、录像），在几种处理器上（大小核、多 CCD、只有一块缓存、
//! 大核很少的低压处理器）：规则对每个程序都一样，检查结果在每种处理器上都合理——
//! 游戏和正在用的从不降级；用户的工作从不节能、从不被挤到小核以外它不需要去的地方，也不占着游戏的大核；
//! 在发声、自己调高优先级的从不降优先级；闲着的不被反复调整。
//!
//! 程序先由用户打开（在前台待过），之后才忙起来——和真实使用一样；一直没被用户碰过、自己在后台忙的
//! 程序照后台的规则让路，另有测试。

use super::*;

/// 几种处理器的核心域（线程数，物理核心数）。
fn layouts() -> Vec<(&'static str, CoreLayout)> {
    vec![
        // i7-12700H：6 大核 12 线程，两组各 4 个小核。
        ("大小核", CoreLayout::with_cores(&[(12, 6), (4, 4), (4, 4)])),
        // 7950X：两个 CCD，各 8 核 16 线程，一样快。
        (
            "双 CCD",
            CoreLayout::with_cores(&[(16, 8), (16, 8)]).with_equal_away(true),
        ),
        // 8 核 16 线程，只有一块缓存。
        ("单缓存", CoreLayout::with_cores(&[(16, 8)])),
        // i5-1235U：2 大核 4 线程，两组各 4 个小核。
        (
            "大核很少",
            CoreLayout::with_cores(&[(4, 2), (4, 4), (4, 4)]),
        ),
    ]
}

fn sim_on(layout: CoreLayout, procs: Vec<Proc>) -> Sim {
    Sim::with(
        ScheduleConfig {
            cores: layout,
            ..ScheduleConfig::default()
        },
        procs,
    )
}

/// 没开过节能，优先级没降到"低于正常"以下（分享时让一步可以），内存与磁盘优先级以外。
fn never_throttled(sim: &Sim, pid: Pid) -> bool {
    sim.actions_on(pid).iter().all(|a| match a {
        Action::SetEcoQos { state, .. } => *state != EcoQos::On,
        Action::SetPriority { class, .. } => class.rank() >= PriorityClass::BelowNormal.rank(),
        _ => true,
    })
}

/// 从没降过优先级。
fn priority_kept(sim: &Sim, pid: Pid) -> bool {
    sim.actions_on(pid).iter().all(|a| match a {
        Action::SetPriority { class, .. } => class.rank() >= PriorityClass::Normal.rank(),
        Action::SetEcoQos { state, .. } => *state != EcoQos::On,
        _ => true,
    })
}

/// 在玩游戏、后台的工作和游戏分处理器时，工作用的核心不包括留给游戏的那些。
fn off_the_games_cores(cores: CoreSet) -> bool {
    !matches!(cores, CoreSet::All | CoreSet::Performance) && !cores.is_performance()
}

/// 玩游戏，后台开着容器（用户打开过 Docker）、编辑器和浏览器闲着。
#[test]
fn a_game_with_containers_and_idle_apps_on_every_kind_of_processor() {
    const GAME: Pid = 1;
    const DOCKER: Pid = 2;
    const BACKEND: Pid = 3;
    const EDITOR: Pid = 4;
    const BROWSER: Pid = 5;
    for (name, layout) in layouts() {
        let mut sim = sim_on(
            layout,
            vec![
                Proc {
                    renders_fullscreen: true,
                    ..app(GAME)
                },
                app(DOCKER),
                child(BACKEND, DOCKER),
                app(EDITOR),
                app(BROWSER),
            ],
        );
        for pid in [DOCKER, EDITOR, BROWSER] {
            sim.foreground = Some(pid);
            sim.run_for(4_000);
        }
        sim.proc_mut(BACKEND).cpu_pct = 40.0;
        sim.proc_mut(GAME).cpu_pct = 250.0;
        sim.foreground = Some(GAME);
        sim.run_for(10 * 60_000);

        assert!(
            sim.never_lowered(GAME),
            "{name}：{:?}",
            sim.actions_on(GAME)
        );
        assert!(
            never_throttled(&sim, BACKEND),
            "{name}：{:?}",
            sim.actions_on(BACKEND)
        );
        let backend = sim.cores_of(BACKEND);
        if layout.partitioned() || layout.can_split(2) {
            assert!(off_the_games_cores(backend), "{name}：容器用着 {backend:?}");
        }
        match name {
            // 大小核、单缓存：大核按物理核心切开。
            "大小核" | "单缓存" => {
                assert!(matches!(backend, CoreSet::Rest(_)), "{name}：{backend:?}")
            }
            // 双 CCD：游戏留着整个 CCD，容器用另一个，不抢缓存。
            "双 CCD" => assert!(backend.is_efficiency(), "{name}：{backend:?}"),
            // 大核只有两个：都给游戏，容器用小核。
            _ => assert!(backend.is_efficiency(), "{name}：{backend:?}"),
        }
        // 闲着的程序：能让到副域就让，只调整一次。
        for idle in [EDITOR, BROWSER] {
            assert!(
                sim.never_lowered_priority(idle),
                "{name}：{:?}",
                sim.actions_on(idle)
            );
            assert!(
                sim.adjustments_per_process()
                    .get(&idle)
                    .copied()
                    .unwrap_or(0)
                    <= 1
            );
        }
    }
}

/// 前台在编译（重点程序），后台在跑训练脚本，浏览器在放歌：编译用优选的大核，训练用其余核心、
/// 不节能；放歌的浏览器不降优先级。
#[test]
fn a_foreground_build_with_training_and_music_in_the_background() {
    const IDE: Pid = 1;
    const TERMINAL: Pid = 2;
    const PYTHON: Pid = 3;
    const BROWSER: Pid = 4;
    const AUDIO: Pid = 5;
    for (name, layout) in layouts() {
        let mut sim = sim_on(
            layout,
            vec![
                app(IDE),
                app(TERMINAL),
                child(PYTHON, TERMINAL),
                app(BROWSER),
                Proc {
                    audio: true,
                    ..child(AUDIO, BROWSER)
                },
            ],
        );
        for pid in [TERMINAL, BROWSER, IDE] {
            sim.foreground = Some(pid);
            sim.run_for(4_000);
        }
        sim.proc_mut(PYTHON).cpu_pct = 600.0;
        sim.proc_mut(AUDIO).cpu_pct = 15.0;
        sim.proc_mut(IDE).cpu_pct = 400.0;
        sim.run_for(8 * 60_000);

        assert!(sim.never_lowered(IDE), "{name}");
        assert!(
            never_throttled(&sim, PYTHON),
            "{name}：{:?}",
            sim.actions_on(PYTHON)
        );
        assert!(
            priority_kept(&sim, AUDIO),
            "{name}：{:?}",
            sim.actions_on(AUDIO)
        );
        assert!(
            priority_kept(&sim, BROWSER),
            "{name}：{:?}",
            sim.actions_on(BROWSER)
        );
    }
}

/// 玩游戏时录像（程序自己把优先级调到"高"）、副屏放着视频（在发声）：都在忙，都是用户的；
/// 让出游戏的大核，但不降优先级，画面和声音不断续。
#[test]
fn a_game_with_recording_and_a_video_on_the_side() {
    const GAME: Pid = 1;
    const RECORDER: Pid = 2;
    const BROWSER: Pid = 3;
    const VIDEO: Pid = 4;
    for (name, layout) in layouts() {
        let mut sim = sim_on(
            layout,
            vec![
                Proc {
                    renders_fullscreen: true,
                    ..app(GAME)
                },
                Proc {
                    own_priority: PriorityClass::High,
                    ..app(RECORDER)
                },
                app(BROWSER),
                Proc {
                    audio: true,
                    ..child(VIDEO, BROWSER)
                },
            ],
        );
        for pid in [RECORDER, BROWSER] {
            sim.foreground = Some(pid);
            sim.run_for(4_000);
        }
        sim.proc_mut(RECORDER).cpu_pct = 60.0;
        sim.proc_mut(VIDEO).cpu_pct = 30.0;
        sim.proc_mut(GAME).cpu_pct = 300.0;
        sim.foreground = Some(GAME);
        sim.run_for(10 * 60_000);

        assert!(sim.never_lowered(GAME), "{name}");
        for pid in [RECORDER, VIDEO] {
            assert!(
                priority_kept(&sim, pid),
                "{name}：{pid} {:?}",
                sim.actions_on(pid)
            );
            if layout.partitioned() {
                assert!(
                    off_the_games_cores(sim.cores_of(pid)),
                    "{name}：{pid} 用着 {:?}",
                    sim.cores_of(pid)
                );
            }
        }
    }
}

/// 什么都不玩：后台编译、容器、训练一起跑，前台只是在看网页。没人和它们抢：全都全速、不节能，
/// 也不互相让——工作和工作之间不分先后。
#[test]
fn several_background_jobs_with_nothing_else_going_on() {
    const BROWSER: Pid = 1;
    for (name, layout) in layouts() {
        let mut procs = vec![app(BROWSER)];
        procs.extend([10, 20, 30].map(app));
        let mut sim = sim_on(layout, procs);
        for pid in [10, 20, 30, BROWSER] {
            sim.foreground = Some(pid);
            sim.run_for(4_000);
        }
        for pid in [10, 20, 30] {
            sim.proc_mut(pid).cpu_pct = 400.0;
        }
        sim.run_for(10 * 60_000);
        for pid in [10, 20, 30] {
            assert_eq!(sim.state_of(pid), Some(Desired::WORK), "{name}：{pid}");
            assert_eq!(
                sim.adjustments_per_process().get(&pid),
                Some(&1),
                "{name}：{pid} {:?}",
                sim.actions_on(pid)
            );
        }
    }
}
