//! 进程分类：按行为和系统属性判断一个进程"是什么"，不维护程序名单。
//!
//! 唯一的名单是安全用途的：反作弊与虚拟机进程，用来确保它们永远不被触碰。

use std::collections::{HashMap, HashSet};

use crate::activity::ActivityTracker;
use crate::known;
use crate::model::{Millis, Pid, ProcessSample, Protection, SessionState, Snapshot};
use crate::prefs::{AppPreference, Preferences};

/// 进程的角色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// 带"关键进程"标记，结束它会蓝屏。
    Critical,
    /// PPL 等受保护进程。
    Protected,
    /// 反作弊程序本身。
    AntiCheat,
    /// 虚拟机工作进程（WSL、Hyper-V）。
    VirtualMachine,
    /// Poise 自己。
    SelfProcess,
    /// 桌面窗口合成器，结束后系统会自动重建。
    Compositor,
    /// 承载 Windows 服务的进程。
    ServiceHost,
    /// 由单一服务拉起的系统辅助进程（例如音频引擎），处理方式是重启该服务。
    ServiceChild { service: String },
    /// 用户会话里的普通程序。
    UserApp,
    /// 其他系统进程。
    SystemOther,
}

/// 分类结果：角色加上与用户感知相关的标记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub role: Role,
    /// 进程树的根（例如浏览器的各个子进程都归到浏览器主进程）。
    pub root_pid: Pid,
    pub in_foreground_tree: bool,
    pub has_visible_window: bool,
    /// 同一进程树里有可见窗口（例如终端里运行的命令行程序、浏览器的辅助进程）。
    /// 这样的进程是某个窗口程序的一部分，重启它会破坏那个程序。
    pub tree_has_window: bool,
    pub plays_audio: bool,
    pub recently_active: bool,
    pub preference: AppPreference,
    /// 是否被识别为游戏（用户标记，或前台全屏且位于游戏目录）。
    pub is_game: bool,
    /// 系统中存在内核级反作弊。此时游戏只允许"任务管理器等价"的操作。
    pub anti_cheat_present: bool,
    /// 可以安全重启的服务：只承载一个服务、接受停止、没有正在运行的依赖者。
    pub restartable_service: Option<String>,
}

impl Classification {
    /// 是否是绝对不能碰的进程。
    pub fn is_untouchable(&self) -> bool {
        matches!(
            self.role,
            Role::Critical
                | Role::Protected
                | Role::AntiCheat
                | Role::VirtualMachine
                | Role::SelfProcess
        )
    }

    /// 是否是调度可以调整的用户程序。
    pub fn is_schedulable(&self) -> bool {
        self.role == Role::UserApp
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClassifyConfig {
    /// "最近在干活"的时间窗口。
    pub active_window_ms: Millis,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            active_window_ms: 3 * 60 * 1000,
        }
    }
}

/// 分类所需的全部上下文。
pub struct ClassifyInput<'a> {
    pub snapshot: &'a Snapshot,
    pub session: &'a SessionState,
    pub activity: &'a ActivityTracker,
    pub preferences: &'a Preferences,
    pub own_pids: &'a [Pid],
    /// 界面代理最近是否上报过会话状态。没有时看不到用户桌面，
    /// 按最保守的方式处理：所有用户程序都视为"有窗口"，清理时一律不碰。
    pub session_fresh: bool,
}

pub struct Classifier {
    cfg: ClassifyConfig,
}

impl Classifier {
    pub fn new(cfg: ClassifyConfig) -> Self {
        Self { cfg }
    }

    pub fn classify_all(&self, input: &ClassifyInput) -> HashMap<Pid, Classification> {
        let snap = input.snapshot;
        let by_pid: HashMap<Pid, &ProcessSample> = snap
            .processes
            .iter()
            .map(|p| (p.facts.key.pid, p))
            .collect();
        let anti_cheat_present = detect_anti_cheat(snap);
        let visible: HashSet<Pid> = input.session.visible_window_pids.iter().copied().collect();
        let audio: HashSet<Pid> = input.session.audio_pids.iter().copied().collect();
        let roots: HashMap<Pid, Pid> = by_pid
            .keys()
            .map(|&pid| (pid, root_of(pid, &by_pid, input.session)))
            .collect();
        let foreground_root = input
            .session
            .foreground_pid
            .and_then(|pid| roots.get(&pid).copied());
        let windowed_roots: HashSet<Pid> = visible
            .iter()
            .filter_map(|pid| roots.get(pid).copied())
            .collect();

        snap.processes
            .iter()
            .map(|p| {
                let pid = p.facts.key.pid;
                let root_pid = roots[&pid];
                let role = role_of(p, &by_pid, input);
                let preference = preference_of(p, root_pid, &by_pid, input.preferences);
                let in_foreground_tree = foreground_root == Some(root_pid);
                let has_visible_window =
                    visible.contains(&pid) || (!input.session_fresh && p.facts.session_id != 0);
                let is_game = role == Role::UserApp
                    && (preference == AppPreference::Game
                        || (in_foreground_tree
                            && input.session.fullscreen
                            && known::looks_like_game_path(p.facts.image_path.as_deref())));
                let c = Classification {
                    root_pid,
                    in_foreground_tree,
                    has_visible_window,
                    tree_has_window: has_visible_window || windowed_roots.contains(&root_pid),
                    plays_audio: audio.contains(&pid),
                    recently_active: input.activity.is_recently_active(
                        &p.facts.key,
                        snap.at,
                        self.cfg.active_window_ms,
                    ),
                    preference,
                    is_game,
                    anti_cheat_present,
                    restartable_service: restartable_service(p, &role, &by_pid),
                    role,
                };
                (pid, c)
            })
            .collect()
    }
}

fn detect_anti_cheat(snap: &Snapshot) -> bool {
    snap.processes.iter().any(|p| {
        known::is_anti_cheat_process(&p.facts.name)
            || p.facts
                .services
                .iter()
                .any(|s| known::is_anti_cheat_service(&s.name))
    })
}

fn role_of(
    p: &ProcessSample,
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
) -> Role {
    let f = &p.facts;
    if input.own_pids.contains(&f.key.pid) {
        return Role::SelfProcess;
    }
    if f.key.pid <= 4 || f.is_critical {
        return Role::Critical;
    }
    if f.protection != Protection::None {
        return Role::Protected;
    }
    if known::is_anti_cheat_process(&f.name)
        || f.services
            .iter()
            .any(|s| known::is_anti_cheat_service(&s.name))
    {
        return Role::AntiCheat;
    }
    if known::is_vm_process(&f.name) {
        return Role::VirtualMachine;
    }
    if f.name == "dwm.exe" && f.session_id != 0 && f.is_in_windows_dir() {
        return Role::Compositor;
    }
    if !f.services.is_empty() {
        return Role::ServiceHost;
    }
    if f.session_id == 0
        && f.is_in_windows_dir()
        && let Some(parent) = f.parent_pid.and_then(|pp| by_pid.get(&pp))
        && is_parent_of(parent, p)
        && let [only] = parent.facts.services.as_slice()
    {
        return Role::ServiceChild {
            service: only.name.clone(),
        };
    }
    if f.session_id != 0 && f.session_id == input.session.session_id {
        return Role::UserApp;
    }
    Role::SystemOther
}

/// 程序自己没有设置偏好时，沿用所在进程树主程序的偏好（辅助进程跟随主程序）。
fn preference_of(
    p: &ProcessSample,
    root_pid: Pid,
    by_pid: &HashMap<Pid, &ProcessSample>,
    prefs: &Preferences,
) -> AppPreference {
    match prefs.lookup(&p.facts) {
        AppPreference::Auto => by_pid
            .get(&root_pid)
            .filter(|root| root.facts.key.pid != p.facts.key.pid)
            .map_or(AppPreference::Auto, |root| prefs.lookup(&root.facts)),
        own => own,
    }
}

fn restartable_service(
    p: &ProcessSample,
    role: &Role,
    by_pid: &HashMap<Pid, &ProcessSample>,
) -> Option<String> {
    let host = match role {
        Role::ServiceHost => p,
        Role::ServiceChild { .. } => by_pid.get(&p.facts.parent_pid?).copied()?,
        _ => return None,
    };
    match host.facts.services.as_slice() {
        [only] if only.accepts_stop && only.running_dependents == 0 => Some(only.name.clone()),
        _ => None,
    }
}

/// 父进程必须比子进程更早创建，否则说明 PID 已被复用。
fn is_parent_of(parent: &ProcessSample, child: &ProcessSample) -> bool {
    parent.facts.key.created <= child.facts.key.created
}

/// 沿父进程向上找到进程树的根：遇到桌面外壳、会话 0、跨会话或 PID 复用时停止。
fn root_of(pid: Pid, by_pid: &HashMap<Pid, &ProcessSample>, session: &SessionState) -> Pid {
    let mut current = pid;
    for _ in 0..32 {
        let Some(proc_) = by_pid.get(&current) else {
            break;
        };
        let Some(parent_pid) = proc_.facts.parent_pid else {
            break;
        };
        let Some(parent) = by_pid.get(&parent_pid) else {
            break;
        };
        let stop = Some(parent_pid) == session.shell_pid
            || parent.facts.session_id == 0
            || parent.facts.session_id != proc_.facts.session_id
            || !is_parent_of(parent, proc_);
        if stop {
            break;
        }
        current = parent_pid;
    }
    current
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessKey, ServiceFacts};

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
        p
    }

    struct World {
        snap: Snapshot,
        session: SessionState,
        activity: ActivityTracker,
        prefs: Preferences,
        own: Vec<Pid>,
        fresh: bool,
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
            }
        }

        fn classify(&self) -> HashMap<Pid, Classification> {
            Classifier::new(ClassifyConfig::default()).classify_all(&ClassifyInput {
                snapshot: &self.snap,
                session: &self.session,
                activity: &self.activity,
                preferences: &self.prefs,
                own_pids: &self.own,
                session_fresh: self.fresh,
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
            running_dependents: 0,
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
    fn anti_cheat_and_vm_are_untouchable() {
        let vgc = proc_(700, None, "vgc.exe", 0);
        let vm = proc_(701, None, "vmmemwsl", 0);
        let c = World::new(vec![vgc, vm]).classify();
        assert!(c[&700].is_untouchable());
        assert!(c[&701].is_untouchable());
        assert!(c[&700].anti_cheat_present);
    }

    #[test]
    fn fullscreen_foreground_in_game_dir_is_a_game() {
        let mut lol = proc_(800, Some(100), "league of legends.exe", 1);
        lol.facts.image_path =
            Some(r"d:\riot games\league of legends\game\league of legends.exe".into());
        let shell = proc_(100, None, "explorer.exe", 1);
        let mut w = World::new(vec![shell, lol]);
        w.session.foreground_pid = Some(800);
        w.session.fullscreen = true;
        assert!(w.classify()[&800].is_game);
        w.session.fullscreen = false;
        assert!(!w.classify()[&800].is_game);
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
}
