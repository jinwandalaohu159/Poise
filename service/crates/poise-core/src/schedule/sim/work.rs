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

/// 复盘"后台在编译，开一局游戏"：大核按物理核心切开，游戏实际用多少就留多少（优选核心在前），
/// 编译用剩下的；游戏要整个大核、或大核被功耗或温度压着时编译只用小核，缓过来才回去；游戏会话结束后
/// 恢复全速。
#[test]
fn replay_a_game_started_while_a_compile_runs_in_the_background() {
    const EDITOR: Pid = 1;
    const COMPILER: Pid = 2;
    const GAME: Pid = 3;
    let mut sim = Sim::new(vec![
        app(EDITOR),
        Proc {
            cpu_pct: 0.0,
            ..child(COMPILER, EDITOR)
        },
        Proc {
            cpu_pct: 250.0,
            renders_fullscreen: true,
            ..app(GAME)
        },
    ]);
    sim.foreground = Some(EDITOR);
    sim.run_for(10_000);
    sim.proc_mut(COMPILER).cpu_pct = 1600.0;
    sim.run_for(30_000);
    sim.foreground = Some(GAME);
    sim.run_for(2 * 60_000);
    // 游戏用两个半核心：优选的 5 个大核留给它，编译用剩下的大核和全部小核。
    let shared = |kept| {
        Some(Desired {
            cores: CoreSet::Rest(kept),
            ..Desired::WORK_SHARED
        })
    };
    assert_eq!(sim.state_of(COMPILER), shared(5));
    // 游戏忙起来：整个大核都给它，编译只用小核。
    sim.proc_mut(GAME).cpu_pct = 600.0;
    sim.run_for(10_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK_ASIDE));
    // 平静下来一分钟后才还回去。
    sim.proc_mut(GAME).cpu_pct = 250.0;
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

    // 游戏退出、会话结束：编译恢复全速。
    sim.procs.retain(|p| p.pid != GAME);
    sim.foreground = None;
    sim.run_for(5 * 60_000);
    assert_eq!(sim.state_of(COMPILER), Some(Desired::WORK));
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
