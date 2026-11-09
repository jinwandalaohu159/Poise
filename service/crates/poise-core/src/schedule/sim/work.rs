//! 用户的程序在干活（编译、训练、准备数据、容器、本地服务）：不论它是什么程序，都按同一条规则——
//! 不当成后台让路、不节能，只在有竞争时分处理器。

use super::*;

/// 从没降过优先级以外的东西：没开节能、没挪到小核、没降内存与磁盘优先级以外（分享时优先级让一步可以）。
fn never_throttled(sim: &Sim, pid: Pid) -> bool {
    sim.actions_on(pid).iter().all(|a| match a {
        Action::SetEcoQos { state, .. } => *state != EcoQos::On,
        Action::SetCoreSet { set, .. } => !set.is_efficiency(),
        _ => true,
    })
}

/// 复盘"在编辑器里点了编译，切去浏览器等它"：编译用全部核心、不让系统节能限速（否则 Windows
/// 把没有前台窗口的编译器放到小核），编译完恢复。
#[test]
fn replay_a_compile_left_running_after_switching_away() {
    const EDITOR: Pid = 1;
    const COMPILER: Pid = 2;
    const BROWSER: Pid = 5;
    let mut sim = Sim::new(vec![
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
        app(BROWSER),
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 1600.0;
    sim.run_for(30_000);
    sim.foreground = Some(BROWSER);
    sim.run_for(10 * 60_000);

    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK));
    assert!(sim.never_lowered(COMPILER));
    assert!(sim.verdicts_of(EDITOR).contains(&Verdict::Working));

    sim.proc_mut(COMPILER).cpu_pct = 0.0;
    sim.run_for(2 * 60_000);
    assert_eq!(sim.verdicts_of(EDITOR).last(), Some(&Verdict::Normal));
    assert_eq!(sim.state_of(COMPILER), None);
}

/// 复盘"在终端里跑起训练脚本、准备数据，然后去看网页"：脚本是用户启动的，不按后台让路；
/// 从没被用户碰过、自己在后台跑的更新程序仍按后台让路，为它让出处理器。
#[test]
fn replay_a_training_script_started_from_a_terminal() {
    const TERMINAL: Pid = 1;
    const PYTHON: Pid = 2;
    const PLOT: Pid = 3;
    const BROWSER: Pid = 5;
    const UPDATER: Pid = 40;
    let mut sim = Sim::new(vec![
        app(TERMINAL),
        app(BROWSER),
        Proc {
            cpu_pct: 60.0,
            ..app(UPDATER)
        },
    ]);
    sim.foreground = Some(TERMINAL);
    sim.run_for(10_000);
    sim.foreground = Some(BROWSER);
    // 脚本在终端里（同一棵进程树），另开了一个画图窗口（自己成一个程序，由终端启动）。
    sim.procs.push(Proc {
        cpu_pct: 600.0,
        ..child(PYTHON, TERMINAL)
    });
    sim.procs.push(Proc {
        cpu_pct: 150.0,
        parent: Some(TERMINAL),
        ..app(PLOT)
    });
    sim.run_for(10 * 60_000);

    assert_eq!(sim.state_of(PYTHON), Some(Desired::WORK));
    assert_eq!(sim.state_of(PLOT), Some(Desired::WORK));
    assert!(sim.never_lowered(PYTHON), "{:?}", sim.actions_on(PYTHON));
    assert!(
        sim.never_lowered(PLOT),
        "{:?} {:?}",
        sim.actions_on(PLOT),
        sim.verdicts_of(PLOT)
    );
    assert!(sim.is_lowered(UPDATER));
    assert!(!sim.verdicts_of(UPDATER).contains(&Verdict::Working));
}

/// 复盘"后台在编译，前台在干重活"（渲染、另一个编译）：大核按物理核心切开，前台实际用多少就留多少
/// （优选核心在前），编译用剩下的；前台要整个大核、或大核被功耗或温度压着时编译只用小核，缓过来才回去；
/// 前台的工作结束后恢复全速。两边都是要吞吐的工作，按核心分是公平的。
#[test]
fn replay_heavy_foreground_work_while_a_compile_runs_in_the_background() {
    const EDITOR: Pid = 1;
    const COMPILER: Pid = 2;
    const RENDER: Pid = 3;
    let mut sim = Sim::new(vec![
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
        Proc {
            cpu_pct: 250.0,
            ..app(RENDER)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 1600.0;
    sim.run_for(30_000);
    sim.foreground = Some(RENDER);
    // 编辑器刚用过的 5 分钟里编译还算正在用：全用。
    sim.run_for(6 * 60_000);
    // 前台用两个半核心：优选的 5 个大核留给它，编译用剩下的大核和全部小核；前台也只用那 5 个，
    // 两边不共用物理核心。
    let shared = |kept| {
        Some(Desired {
            cores: CoreSet::Rest(kept),
            ..Desired::WORK_SHARED
        })
    };
    assert_eq!(sim.state_of(COMPILER), shared(5));
    assert_eq!(sim.cores_of(RENDER), CoreSet::Preferred(5));
    // 前台忙起来：整个大核都给它，编译只用小核。
    sim.proc_mut(RENDER).cpu_pct = 600.0;
    sim.run_for(10_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_ASIDE));
    // 不再切开：前台用整个主域，要得多时还加上副域。
    let render = sim.cores_of(RENDER);
    assert!(
        render.is_performance() && !matches!(render, CoreSet::Preferred(_)),
        "{render:?}"
    );
    // 平静下来一分钟后才还回去。
    sim.proc_mut(RENDER).cpu_pct = 250.0;
    sim.run_for(30_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_ASIDE));
    sim.run_for(2 * 60_000);
    assert_eq!(sim.state_of(COMPILER), shared(5));

    // 大核被功耗或温度压着：编译只用小核，频率回升后至少再等两分钟才回来。
    sim.cpu_held = true;
    sim.run_for(10_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_ASIDE));
    sim.cpu_held = false;
    sim.run_for(60_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_ASIDE));
    sim.run_for(2 * 60_000);
    assert_eq!(sim.state_of(COMPILER), shared(5));

    // 前台的工作结束、关掉：编译恢复全速。
    sim.procs.retain(|p| p.pid != RENDER);
    sim.foreground = None;
    sim.run_for(5 * 60_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK));
}

/// 主域切开时重点程序用的核心跟着它实际要的走：忙起来立即多留（不会被关在不够的核心里），
/// 后台的工作结束、不再切开时回到整个主域。
#[test]
fn the_focus_widens_at_once_when_it_needs_more_of_a_split_main_domain() {
    const EDITOR: Pid = 1;
    const COMPILER: Pid = 2;
    const RENDER: Pid = 3;
    let mut sim = Sim::new(vec![
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
        Proc {
            cpu_pct: 150.0,
            ..app(RENDER)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 1600.0;
    sim.run_for(30_000);
    sim.foreground = Some(RENDER);
    sim.run_for(6 * 60_000);
    let CoreSet::Preferred(narrow) = sim.cores_of(RENDER) else {
        panic!("{:?}", sim.cores_of(RENDER));
    };
    assert_eq!(sim.cores_of(COMPILER), CoreSet::Rest(narrow));
    // 一下子要多一个半核心：下一轮就多留。
    sim.proc_mut(RENDER).cpu_pct = 300.0;
    sim.run_for(4_000);
    let CoreSet::Preferred(wide) = sim.cores_of(RENDER) else {
        panic!("{:?}", sim.cores_of(RENDER));
    };
    assert!(wide > narrow, "{narrow} → {wide}");
    assert_eq!(sim.cores_of(COMPILER), CoreSet::Rest(wide));
    // 编译结束：不再切开，前台回到整个主域。
    sim.proc_mut(COMPILER).cpu_pct = 0.0;
    sim.run_for(2 * 60_000);
    assert_eq!(sim.cores_of(RENDER), CoreSet::Domains(1));
}

/// 用电池时干活的程序不限核心（只降优先级），主域没有真的切开：重点程序不被关进优选核心，照旧用整个主域。
#[test]
fn on_battery_the_focus_is_not_confined_to_a_split_that_is_not_in_effect() {
    const EDITOR: Pid = 1;
    const COMPILER: Pid = 2;
    const RENDER: Pid = 3;
    let mut sim = Sim::new(vec![
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
        Proc {
            cpu_pct: 150.0,
            ..app(RENDER)
        },
    ]);
    sim.on_battery = true;
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 1600.0;
    sim.run_for(30_000);
    sim.foreground = Some(RENDER);
    sim.run_for(6 * 60_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::LOWERED));
    assert!(
        !matches!(sim.cores_of(RENDER), CoreSet::Preferred(_)),
        "{:?}",
        sim.cores_of(RENDER)
    );
}

/// 复盘一局英雄联盟（0.8.4 实测帧数稳不住）：用户在客户端里排队、选英雄，客户端整局都在后台占一个半
/// 核心；后台还在编译。游戏只靠一两个主线程，帧数取决于频率，频率取决于共用的功耗——游戏开着的时候，
/// 客户端和编译都在小核上节能运行，一个大核也不分给它们；游戏结束后编译回到全速，客户端回到前台照常。
/// 用户设为"始终优先"的程序在后台忙，照用户的不节能。
#[test]
fn replay_a_game_while_its_client_and_a_compile_stay_busy() {
    const CLIENT: Pid = 1;
    const GAME: Pid = 2;
    const EDITOR: Pid = 3;
    const COMPILER: Pid = 4;
    const ENGINE: Pid = 5;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            preference: AppPreference::AlwaysFast,
            ..app(ENGINE)
        },
        Proc {
            cpu_pct: 150.0,
            ..app(CLIENT)
        },
        Proc {
            cpu_pct: 0.0,
            ..app(GAME)
        },
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 800.0;
    sim.foreground = Some(CLIENT);
    sim.run_for(60_000);
    // 进入对局：游戏到了前台，像游戏一样全屏渲染。
    {
        let game = sim.proc_mut(GAME);
        game.cpu_pct = 150.0;
        game.renders_fullscreen = true;
    }
    sim.foreground = Some(GAME);
    sim.run_for(3 * 60_000);
    assert_eq!(
        sim.state_of(GAME).map(|d| d.cores),
        Some(CoreSet::Performance)
    );
    assert_eq!(sim.state_of(CLIENT), Some(Desired::WORK_YIELDED));
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_YIELDED));
    // 编译（按编辑器这个程序记）和客户端开局前就在干活，判定一直是"在后台干活"：换成为游戏让路照样记下来。
    let reported_gaming = |pid| {
        sim.decisions.iter().any(|d| {
            matches!(
                d,
                Decision::Program { root, state, .. }
                    if root.pid == pid && state.cause == Some(Cause::Gaming)
            )
        })
    };
    assert!(
        reported_gaming(EDITOR) && reported_gaming(CLIENT),
        "{:?}",
        sim.decisions
    );
    assert!(
        sim.actions_on(ENGINE).iter().all(|a| !matches!(
            a,
            Action::SetEcoQos {
                state: EcoQos::On,
                ..
            }
        )),
        "{:?}",
        sim.actions_on(ENGINE)
    );
    // 对局中切出去看一眼客户端：它在前台时照常，切回游戏又回到小核节能。
    sim.foreground = Some(CLIENT);
    sim.run_for(4_000);
    assert_eq!(sim.state_of(CLIENT), Some(Desired::WORK));
    sim.foreground = Some(GAME);
    sim.run_for(10_000);
    assert_eq!(sim.state_of(CLIENT), Some(Desired::WORK_YIELDED));
    // 游戏退出、会话结束，回到客户端的大厅（"始终优先"的程序也关了）：编译恢复全速，客户端照常。
    sim.procs.retain(|p| p.pid != GAME && p.pid != ENGINE);
    sim.proc_mut(CLIENT).cpu_pct = 20.0;
    sim.foreground = Some(CLIENT);
    sim.run_for(5 * 60_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK));
    assert_eq!(sim.state_of(CLIENT), Some(Desired::WORK));
}

/// 复盘"后台有容器在跑数据管道，前台在干重活"：用户打开过 Docker Desktop，它的网络与文件代理是用户的、
/// 在干活：不节能、不挪到小核；前台在干重活时，优选的大核留给前台，代理用其余核心、优先级让一步。
#[test]
fn replay_containers_running_while_the_user_works() {
    const DOCKER: Pid = 1;
    const BACKEND: Pid = 2;
    const EDITOR: Pid = 5;
    let mut sim = Sim::new(vec![
        app(DOCKER),
        Proc {
            cpu_pct: 30.0,
            ..child(BACKEND, DOCKER)
        },
        Proc {
            cpu_pct: 150.0,
            ..app(EDITOR)
        },
    ]);
    sim.foreground = Some(DOCKER);
    sim.run_for(4_000);
    sim.foreground = Some(EDITOR);
    // 刚用过的 5 分钟里还算正在用：全用。
    sim.run_for(7 * 60_000);

    assert!(
        never_throttled(&sim, BACKEND),
        "{:?}",
        sim.actions_on(BACKEND)
    );
    assert_eq!(
        sim.state_of(BACKEND),
        Some(Desired {
            cores: CoreSet::Rest(4),
            ..Desired::WORK_SHARED
        })
    );
    assert_eq!(sim.cores_of(EDITOR), CoreSet::Preferred(4));
    assert!(sim.verdicts_of(DOCKER).contains(&Verdict::Working));
}

/// 复盘"在浏览器里和本地的大模型聊天"：干活的是一个用户从没点开过的托盘程序，浏览器经本机网络连着它。
/// 它在为正在用的浏览器干活，不按后台让路；同时在跑、没人连着的更新程序照样让路。
#[test]
fn replay_a_local_model_answering_the_browser() {
    const BROWSER: Pid = 1;
    const OLLAMA: Pid = 2;
    const UPDATER: Pid = 40;
    let mut sim = Sim::new(vec![
        app(BROWSER),
        Proc {
            cpu_pct: 400.0,
            ..app(OLLAMA)
        },
        Proc {
            cpu_pct: 60.0,
            ..app(UPDATER)
        },
    ]);
    sim.links = vec![crate::model::LocalLink {
        client: BROWSER,
        server: OLLAMA,
    }];
    sim.foreground = Some(BROWSER);
    sim.run_for(3 * 60_000);

    assert!(sim.never_lowered(OLLAMA), "{:?}", sim.actions_on(OLLAMA));
    assert_eq!(sim.state_of(OLLAMA), Some(Desired::WORK));
    assert!(sim.verdicts_of(OLLAMA).contains(&Verdict::Working));
    assert!(sim.is_lowered(UPDATER));
}

/// 复盘"用显卡训练模型，切去看网页"：训练脚本自己只用不到半个核心给显卡喂数据。用户的程序在忙就是在干活，
/// 不看它忙的是处理器还是显卡——喂数据的线程被挤到小核、压低频率，显卡就跟着空等。
#[test]
fn replay_a_script_feeding_the_graphics_card() {
    const TERMINAL: Pid = 1;
    const PYTHON: Pid = 2;
    const BROWSER: Pid = 5;
    let mut sim = Sim::new(vec![app(TERMINAL), app(BROWSER)]);
    sim.foreground = Some(TERMINAL);
    sim.run_for(10_000);
    sim.foreground = Some(BROWSER);
    sim.procs.push(Proc {
        cpu_pct: 45.0,
        ..child(PYTHON, TERMINAL)
    });
    sim.run_for(8 * 60_000);

    assert_eq!(sim.state_of(PYTHON), Some(Desired::WORK));
    assert!(sim.never_lowered(PYTHON), "{:?}", sim.actions_on(PYTHON));
    assert!(sim.verdicts_of(TERMINAL).contains(&Verdict::Working));
}
