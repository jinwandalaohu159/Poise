//! 进程分类的测试。

use super::*;
use crate::model::{ProcessFacts, ProcessKey, ServiceFacts};
use crate::model::{ProcessSample, Protection};

pub fn proc_(pid: Pid, parent: Option<Pid>, name: &str, session: u32) -> ProcessSample {
    ProcessSample {
        facts: ProcessFacts {
            key: ProcessKey {
                pid,
                created: pid as u64,
            },
            parent_pid: parent,
            name: name.into(),
            image_path: Some(format!(r"c:\apps\{name}")),
            session_id: session,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn system(mut p: ProcessSample) -> ProcessSample {
    p.facts.image_path = Some(format!(r"c:\windows\system32\{}", p.facts.name));
    p.facts.in_windows_dir = true;
    p
}

struct World {
    snap: Snapshot,
    session: SessionState,
    activity: ActivityTracker,
    prefs: Preferences,
    own: Vec<Pid>,
    fresh: bool,
    gpu: Option<(Pid, f32)>,
    games: Vec<Pid>,
}

impl World {
    fn new(processes: Vec<ProcessSample>) -> Self {
        Self {
            snap: Snapshot {
                at: 1,
                processes,
                ..Default::default()
            },
            session: SessionState {
                session_id: 1,
                shell_pid: Some(100),
                ..Default::default()
            },
            activity: ActivityTracker::default(),
            prefs: Preferences::default(),
            own: vec![],
            fresh: true,
            gpu: None,
            games: vec![],
        }
    }

    fn classify(&self) -> HashMap<Pid, Classification> {
        self.classify_with(&mut Classifier::new(ClassifyConfig::default()))
    }

    /// 用同一个分类器连续分类（它记得上一轮的进程树）。
    fn classify_with(&self, classifier: &mut Classifier) -> HashMap<Pid, Classification> {
        classifier.classify_all(&ClassifyInput {
            snapshot: &self.snap,
            session: &self.session,
            activity: &self.activity,
            preferences: &self.prefs,
            own_pids: &self.own,
            session_fresh: self.fresh,
            foreground_gpu: self.gpu,
            active_games: &self.games,
        })
    }
}

#[test]
fn critical_and_protected_detected_by_flags() {
    let mut crit = proc_(500, None, "csrss.exe", 1);
    crit.facts.is_critical = true;
    let mut ppl = proc_(501, None, "msmpeng.exe", 0);
    ppl.facts.protection = Protection::Light;
    let c = World::new(vec![crit, ppl]).classify();
    assert_eq!(c[&500].role, Role::Critical);
    assert_eq!(c[&501].role, Role::Protected);
}

#[test]
fn service_child_is_attributed_to_its_service() {
    let mut host = system(proc_(600, None, "svchost.exe", 0));
    host.facts.services = vec![ServiceFacts {
        name: "Audiosrv".into(),
        accepts_stop: true,
        ..Default::default()
    }];
    let child = system(proc_(601, Some(600), "audiodg.exe", 0));
    let c = World::new(vec![host, child]).classify();
    assert_eq!(c[&600].role, Role::ServiceHost);
    assert_eq!(
        c[&601].role,
        Role::ServiceChild {
            service: "Audiosrv".into()
        }
    );
}

#[test]
fn child_of_multi_service_host_is_not_attributed() {
    let mut host = system(proc_(600, None, "svchost.exe", 0));
    host.facts.services = vec![
        ServiceFacts {
            name: "A".into(),
            ..Default::default()
        },
        ServiceFacts {
            name: "B".into(),
            ..Default::default()
        },
    ];
    let child = system(proc_(601, Some(600), "helper.exe", 0));
    let c = World::new(vec![host, child]).classify();
    assert_eq!(c[&601].role, Role::SystemOther);
}

/// 终端里跑的命令行工具拉起了编译服务进程，然后自己退出了：服务进程仍归终端这个程序。
/// 终端也关了，它才自成一个程序。
#[test]
fn orphaned_workers_stay_with_the_program_that_started_them() {
    let shell = proc_(100, None, "explorer.exe", 1);
    let terminal = proc_(200, Some(100), "terminal.exe", 1);
    let cli = proc_(201, Some(200), "dotnet.exe", 1);
    let worker = proc_(202, Some(201), "dotnet.exe", 1);
    let mut classifier = Classifier::new(ClassifyConfig::default());
    let w = World::new(vec![shell.clone(), terminal.clone(), cli, worker.clone()]);
    assert_eq!(w.classify_with(&mut classifier)[&202].root_pid, 200);

    let w = World::new(vec![shell.clone(), terminal, worker.clone()]);
    assert_eq!(w.classify_with(&mut classifier)[&202].root_pid, 200);
    // 再下一轮依然记得。
    assert_eq!(w.classify_with(&mut classifier)[&202].root_pid, 200);

    let w = World::new(vec![shell, worker]);
    assert_eq!(w.classify_with(&mut classifier)[&202].root_pid, 202);
}

#[test]
fn browser_children_share_the_foreground_root() {
    let shell = proc_(100, None, "explorer.exe", 1);
    let browser = proc_(200, Some(100), "chrome.exe", 1);
    let renderer = proc_(201, Some(200), "chrome.exe", 1);
    let other = proc_(300, Some(100), "notepad.exe", 1);
    let mut w = World::new(vec![shell, browser, renderer, other]);
    w.session.foreground_pid = Some(201);
    let c = w.classify();
    assert!(c[&200].in_foreground_tree);
    assert!(c[&201].in_foreground_tree);
    assert!(!c[&300].in_foreground_tree);
    assert_eq!(c[&201].root_pid, 200);
}

#[test]
fn independent_programs_are_not_grouped_under_their_launcher() {
    let at = |mut p: ProcessSample, path: &str| {
        p.facts.image_path = Some(path.into());
        p
    };
    let shell = proc_(100, None, "explorer.exe", 1);
    let chat = at(
        proc_(200, Some(100), "buzzdesk.exe", 1),
        r"d:\buzzdesk\buzzdesk.exe",
    );
    // 聊天软件里点链接打开的浏览器（有窗口、装在别处）。
    let browser = at(
        proc_(201, Some(200), "chrome.exe", 1),
        r"c:\chrome\chrome.exe",
    );
    let renderer = at(
        proc_(202, Some(201), "chrome.exe", 1),
        r"c:\chrome\chrome.exe",
    );
    // 聊天软件内嵌的网页视图（没有自己的顶层窗口）。
    let webview = at(
        proc_(203, Some(200), "msedgewebview2.exe", 1),
        r"c:\webview\msedgewebview2.exe",
    );
    // 正在输入时被拉起的输入法。
    let ime = at(
        proc_(204, Some(203), "imewatchdog.exe", 1),
        r"c:\ime\imewatchdog.exe",
    );
    let steam = at(proc_(300, Some(100), "steam.exe", 1), r"d:\steam\steam.exe");
    // 同一程序的界面进程：有窗口，但装在主程序目录下。
    let ui = at(
        proc_(301, Some(300), "steamwebhelper.exe", 1),
        r"d:\steam\bin\steamwebhelper.exe",
    );
    // 从启动器里打开、正在全屏玩的游戏：装在启动器目录下，但全屏窗口属于它自己。
    let game = at(
        proc_(302, Some(300), "game.exe", 1),
        r"d:\steam\steamapps\common\g\game.exe",
    );
    let mut w = World::new(vec![
        shell, chat, browser, renderer, webview, ime, steam, ui, game,
    ]);
    w.session.visible_window_pids = vec![200, 201, 300, 301, 302];
    w.session.foreground_pid = Some(302);
    w.session.fullscreen = true;
    let c = w.classify();
    assert_eq!(c[&201].root_pid, 201);
    assert_eq!(c[&202].root_pid, 201);
    assert_eq!(c[&203].root_pid, 200);
    assert_eq!(c[&204].root_pid, 204);
    assert_eq!(c[&301].root_pid, 300);
    assert_eq!(c[&302].root_pid, 302);
}

#[test]
fn reused_pid_does_not_link_to_new_parent() {
    // 父 PID 500 被一个更晚创建的进程复用了。
    let mut child = proc_(400, Some(500), "app.exe", 1);
    child.facts.key.created = 10;
    let mut impostor = proc_(500, None, "other.exe", 1);
    impostor.facts.key.created = 20;
    let c = World::new(vec![child, impostor]).classify();
    assert_eq!(c[&400].root_pid, 400);
}

#[test]
fn schedule_scope_excludes_system_components_and_admits_chores() {
    let app = proc_(900, None, "baidunetdisk.exe", 1);
    let shell = system(proc_(901, None, "textinputhost.exe", 1));
    let ime = proc_(902, None, "sogouimebroker.exe", 1);
    let mut popup = proc_(905, None, "popblock.exe", 1);
    popup.facts.image_path = Some(r"d:\program files\huorong\sysdiag\bin\popblock.exe".into());
    let mut indexer = system(proc_(903, None, "searchindexer.exe", 0));
    indexer.facts.services = vec![ServiceFacts {
        name: "WSearch".into(),
        ..Default::default()
    }];
    let svchost = system(proc_(904, None, "svchost.exe", 0));
    let service = |pid, names: &[&str]| {
        let mut p = system(proc_(pid, None, "svchost.exe", 0));
        p.facts.services = names
            .iter()
            .map(|n| ServiceFacts {
                name: (*n).into(),
                ..Default::default()
            })
            .collect();
        p
    };
    let update = service(906, &["wuauserv"]);
    let shared = service(907, &["wuauserv", "Audiosrv"]);
    let prefetch = service(908, &["SysMain"]);
    let c = World::new(vec![
        app, shell, ime, indexer, svchost, popup, update, shared, prefetch,
    ])
    .classify();
    // 系统服务（包括更新、索引这类后台杂活）一律交给系统。
    assert_eq!(c[&906].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&907].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&908].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&905].schedule, ScheduleScope::Excluded);
    assert!(c[&900].is_schedulable());
    assert_eq!(c[&901].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&902].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&903].schedule, ScheduleScope::Excluded);
    assert_eq!(c[&904].schedule, ScheduleScope::Excluded);
}

#[test]
fn anti_cheat_and_vm_are_untouchable() {
    let vgc = proc_(700, None, "vgc.exe", 0);
    let vm = proc_(701, None, "vmmemwsl", 0);
    let c = World::new(vec![vgc, vm]).classify();
    assert!(c[&700].is_untouchable());
    assert!(c[&701].is_untouchable());
    assert!(c[&700].anti_cheat_present);
}

/// 同一套反作弊的其他程序（不在名单上）也不碰：与名单里的程序同目录，或装在已知的反作弊目录里。
#[test]
fn every_program_of_an_anti_cheat_suite_is_untouchable() {
    let in_dir = |pid, name: &str, dir: &str| {
        let mut p = proc_(pid, None, name, 1);
        p.facts.image_path = Some(format!(r"{dir}\{name}"));
        p
    };
    let c = World::new(vec![
        in_dir(700, "vgc.exe", r"c:\program files\riot vanguard"),
        in_dir(701, "vgx.exe", r"c:\program files\riot vanguard"),
        in_dir(702, "helper.exe", r"d:\games\some game\easyanticheat"),
        in_dir(703, "vgx.exe", r"c:\apps"),
    ])
    .classify();
    assert!(c[&701].is_untouchable());
    assert!(c[&702].is_untouchable());
    // 同名但装在别处的程序不受影响。
    assert!(!c[&703].is_untouchable());
}

#[test]
fn fullscreen_window_that_renders_is_a_game_wherever_it_is_installed() {
    let mut game = proc_(800, Some(100), "anything.exe", 1);
    game.facts.image_path = Some(r"e:\stuff\anything.exe".into());
    let shell = proc_(100, None, "explorer.exe", 1);
    let mut w = World::new(vec![shell, game]);
    w.session.foreground_pid = Some(800);
    w.session.fullscreen = true;
    w.gpu = Some((800, 65.0));
    assert!(w.classify()[&800].is_game);
    // 不全屏、不渲染（全屏视频、演示文稿）、测到的是别的进程：都不算。
    w.session.fullscreen = false;
    assert!(!w.classify()[&800].is_game);
    w.session.fullscreen = true;
    w.gpu = Some((800, 4.0));
    assert!(!w.classify()[&800].is_game);
    w.gpu = Some((801, 65.0));
    assert!(!w.classify()[&800].is_game);
}

#[test]
fn the_client_that_started_a_game_is_not_a_game() {
    let mut client = proc_(810, None, "client.exe", 1);
    client.facts.image_path = Some(r"d:\launcher\client.exe".into());
    let mut game = proc_(811, Some(810), "game.exe", 1);
    game.facts.image_path = Some(r"d:\launcher\game\game.exe".into());
    let mut w = World::new(vec![client, game]);
    w.session.foreground_pid = Some(811);
    w.session.fullscreen = true;
    w.gpu = Some((811, 80.0));
    let c = w.classify();
    assert!(c[&811].is_game);
    assert!(!c[&810].is_game);
    assert_eq!(c[&811].root_pid, 811);
    // 已确认的游戏切到后台：仍是独立的程序，不继承客户端的偏好。
    w.session.foreground_pid = Some(810);
    w.session.fullscreen = false;
    w.games = vec![811];
    assert_eq!(w.classify()[&811].root_pid, 811);
    w.session.fullscreen = true;
    // 客户端自己的窗口铺满屏幕，但渲染在别的子进程里：不算游戏。
    w.session.foreground_pid = Some(810);
    w.gpu = Some((810, 1.0));
    assert!(!w.classify()[&810].is_game);
}

/// 无边框窗口、没被自动识别的游戏：用户单独给游戏本体选"游戏"后，它成为独立的程序，客户端不受影响。
#[test]
fn a_child_with_its_own_preference_is_its_own_program() {
    let mut client = proc_(820, None, "leagueclient.exe", 1);
    client.facts.image_path = Some(r"d:\games\lol\leagueclient.exe".into());
    let mut game = proc_(821, Some(820), "league of legends.exe", 1);
    game.facts.image_path = Some(r"d:\games\lol\game\league of legends.exe".into());
    let mut w = World::new(vec![client, game]);
    w.session.visible_window_pids = vec![820, 821];
    assert_eq!(w.classify()[&821].root_pid, 820);
    w.prefs.set(
        r"d:\games\lol\game\league of legends.exe",
        AppPreference::Game,
    );
    let c = w.classify();
    assert_eq!(c[&821].root_pid, 821);
    assert!(c[&821].is_game);
    assert!(!c[&820].is_game);
    assert_eq!(c[&820].preference, AppPreference::Auto);
}

#[test]
fn user_marked_game_is_a_game_even_in_background() {
    let app = proc_(900, None, "emu.exe", 1);
    let mut w = World::new(vec![app]);
    w.prefs.set(r"c:\apps\emu.exe", AppPreference::Game);
    assert!(w.classify()[&900].is_game);
}

#[test]
fn stale_session_treats_user_processes_as_windowed() {
    let app = proc_(900, None, "helper.exe", 1);
    let svc = proc_(901, None, "svc.exe", 0);
    let mut w = World::new(vec![app, svc]);
    assert!(!w.classify()[&900].has_visible_window);
    w.fresh = false;
    let c = w.classify();
    assert!(c[&900].has_visible_window);
    assert!(!c[&901].has_visible_window);
}

#[test]
fn windowless_child_of_windowed_app_belongs_to_its_tree() {
    // 终端（有窗口）里运行的命令行程序（没有窗口）。
    let shell = proc_(100, None, "explorer.exe", 1);
    let terminal = proc_(200, Some(100), "windowsterminal.exe", 1);
    let cli = proc_(201, Some(200), "node.exe", 1);
    let lone = proc_(300, Some(100), "updater.exe", 1);
    let mut w = World::new(vec![shell, terminal, cli, lone]);
    w.session.visible_window_pids = vec![200];
    let c = w.classify();
    assert!(!c[&201].has_visible_window);
    assert!(c[&201].tree_has_window);
    assert!(!c[&300].tree_has_window);
}

#[test]
fn helper_inherits_the_main_app_preference() {
    let shell = proc_(100, None, "explorer.exe", 1);
    let main = proc_(200, Some(100), "netdisk.exe", 1);
    let helper = proc_(201, Some(200), "netdiskhelper.exe", 1);
    let mut w = World::new(vec![shell, main, helper]);
    w.prefs
        .set(r"c:\apps\netdisk.exe", AppPreference::Background);
    let c = w.classify();
    assert_eq!(c[&201].preference, AppPreference::Background);
    w.prefs
        .set(r"c:\apps\netdiskhelper.exe", AppPreference::AlwaysFast);
    assert_eq!(w.classify()[&201].preference, AppPreference::AlwaysFast);
}

#[test]
fn own_process_is_self() {
    let me = proc_(42, None, "poise-service.exe", 0);
    let mut w = World::new(vec![me]);
    w.own = vec![42];
    assert_eq!(w.classify()[&42].role, Role::SelfProcess);
}
