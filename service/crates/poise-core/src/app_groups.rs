//! 程序偏好页"正在运行"的分组：像任务管理器一样，同一程序的进程归到主程序名下。
//!
//! 分组沿用分类时的进程树（[`Classification::root_pid`]）：辅助进程归到主程序，独立程序单独成组。
//! 同一路径的多棵进程树（同一程序开了两次）合并为一组。

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::classify::{Classification, Role};
use crate::model::{Pid, ProcessSample, Snapshot};

/// 这一组在程序偏好页的哪一栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// 用户程序：可以选择类别。
    App,
    /// 用户会话里不参与调度的程序（输入法、驱动助手、安全软件等）。
    Excluded,
}

/// 正在运行的一个程序。
#[derive(Debug)]
pub struct AppGroup<'a> {
    pub path: &'a str,
    /// 主进程（进程树的根）。
    pub main: &'a ProcessSample,
    pub class: &'a Classification,
    pub kind: GroupKind,
    pub is_game: bool,
    /// 组内全部进程及其分类，主进程在前，其余按名称、进程号排序。
    pub members: Vec<(&'a ProcessSample, &'a Classification)>,
}

/// 正在运行、值得列出的程序，按键排序（小写路径；服务进程再加上 `|服务名`）：
/// - 用户程序：有窗口、最近在干活，或 `remembered` 里记着的（任一进程满足即可），连同它的全部进程；
/// - 白名单里的后台杂活（svchost 按服务分开）；
/// - 用户会话里不参与调度的程序。
///
/// Windows 目录下的系统组件不单独列出；作为某个程序的子进程时（如 conhost）归到该程序名下。
pub fn running_groups<'a>(
    snapshot: &'a Snapshot,
    classes: &'a HashMap<Pid, Classification>,
    remembered: &dyn Fn(&str) -> bool,
) -> BTreeMap<String, AppGroup<'a>> {
    let by_pid = snapshot.by_pid();
    let user_program = |p: &ProcessSample| {
        classes
            .get(&p.facts.key.pid)
            .is_some_and(|c| c.role == Role::UserApp)
            && p.facts.image_path.is_some()
            && !p.facts.in_windows_dir
    };
    // 进程所属的主程序：进程树的根是用户程序时归到根，否则自己是用户程序时归到自己。
    let owner = |p: &'a ProcessSample, c: &Classification| -> Option<&'a ProcessSample> {
        match by_pid.get(&c.root_pid).copied() {
            Some(root) if user_program(root) => Some(root),
            _ => user_program(p).then_some(p),
        }
    };

    let mut groups: BTreeMap<String, AppGroup> = BTreeMap::new();
    // 第一遍：哪些程序要列出。
    for p in &snapshot.processes {
        let Some(c) = classes.get(&p.facts.key.pid) else {
            continue;
        };
        let Some(app) = owner(p, c) else {
            continue;
        };
        let Some(path) = &app.facts.image_path else {
            continue;
        };
        if !(c.has_visible_window || c.recently_active || remembered(path)) {
            continue;
        }
        let class = classes.get(&app.facts.key.pid).unwrap_or(c);
        let kind = if class.is_schedulable() {
            GroupKind::App
        } else {
            GroupKind::Excluded
        };
        groups
            .entry(path.clone())
            .or_insert_with(|| group(path, app, class, kind))
            .is_game |= c.is_game;
    }
    // 第二遍：程序的全部进程（包括不满足列出条件的辅助进程）。
    for p in &snapshot.processes {
        let Some(c) = classes.get(&p.facts.key.pid) else {
            continue;
        };
        let Some(path) = owner(p, c).and_then(|app| app.facts.image_path.as_ref()) else {
            continue;
        };
        if let Some(g) = groups.get_mut(path) {
            g.members.push((p, c));
        }
    }
    for g in groups.values_mut() {
        let main = g.main.facts.key.pid;
        g.members.sort_by_cached_key(|(p, _)| {
            (
                p.facts.key.pid != main,
                p.facts.name.to_lowercase(),
                p.facts.key.pid,
            )
        });
    }
    groups
}

impl AppGroup<'_> {
    /// 组里可以单独设置类别的进程：有自己窗口（或在玩）的另一个程序文件，例如客户端里的游戏本体。
    /// 主程序跟随这一组的类别；同一个程序文件只取第一个进程。
    pub fn separable_members(&self) -> HashSet<Pid> {
        let mut seen: HashSet<&str> = HashSet::from([self.path]);
        self.members
            .iter()
            .filter(|(p, c)| {
                c.is_schedulable()
                    && (c.has_visible_window || c.is_game)
                    && p.facts
                        .image_path
                        .as_deref()
                        .is_some_and(|path| seen.insert(path))
            })
            .map(|(p, _)| p.facts.key.pid)
            .collect()
    }
}

fn group<'a>(
    path: &'a str,
    main: &'a ProcessSample,
    class: &'a Classification,
    kind: GroupKind,
) -> AppGroup<'a> {
    AppGroup {
        path,
        main,
        class,
        kind,
        is_game: false,
        members: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::ScheduleScope;
    use crate::model::{ProcessFacts, ProcessKey};
    use crate::prefs::AppPreference;

    fn proc(pid: Pid, name: &str, path: Option<&str>) -> ProcessSample {
        ProcessSample {
            facts: ProcessFacts {
                key: ProcessKey { pid, created: 1 },
                name: name.into(),
                image_path: path.map(String::from),
                in_windows_dir: path.is_some_and(|p| p.starts_with(r"c:\windows\")),
                ..Default::default()
            },
            metrics: Default::default(),
        }
    }

    fn class(root_pid: Pid, window: bool) -> Classification {
        Classification {
            role: Role::UserApp,
            root_pid,
            in_foreground_tree: false,
            has_visible_window: window,
            tree_has_window: window,
            plays_audio: false,
            emits_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: None,
            schedule: ScheduleScope::App,
        }
    }

    struct World {
        snapshot: Snapshot,
        classes: HashMap<Pid, Classification>,
    }

    impl World {
        fn new() -> Self {
            Self {
                snapshot: Snapshot::default(),
                classes: HashMap::new(),
            }
        }

        fn add(&mut self, p: ProcessSample, c: Classification) -> &mut Self {
            self.classes.insert(p.facts.key.pid, c);
            self.snapshot.processes.push(p);
            self
        }

        fn groups(&self) -> BTreeMap<String, AppGroup<'_>> {
            running_groups(&self.snapshot, &self.classes, &|_| false)
        }
    }

    fn pids(g: &AppGroup) -> Vec<Pid> {
        g.members.iter().map(|(p, _)| p.facts.key.pid).collect()
    }

    #[test]
    fn other_programs_with_their_own_window_can_be_set_separately() {
        let mut w = World::new();
        w.add(
            proc(10, "client.exe", Some(r"c:\riot\client.exe")),
            class(10, true),
        )
        .add(
            proc(11, "client.exe", Some(r"c:\riot\client.exe")),
            class(10, true),
        )
        .add(
            proc(12, "game.exe", Some(r"c:\riot\game\game.exe")),
            class(10, true),
        )
        .add(
            proc(13, "game.exe", Some(r"c:\riot\game\game.exe")),
            class(10, true),
        )
        .add(
            proc(14, "helper.exe", Some(r"c:\riot\helper.exe")),
            class(10, false),
        );
        let groups = w.groups();
        let g = &groups[r"c:\riot\client.exe"];
        assert_eq!(g.separable_members(), HashSet::from([12]));
    }

    #[test]
    fn helpers_are_listed_under_the_main_program() {
        let mut w = World::new();
        w.add(
            proc(
                10,
                "docker desktop.exe",
                Some(r"c:\docker\docker desktop.exe"),
            ),
            class(10, true),
        )
        .add(
            proc(
                12,
                "com.docker.backend.exe",
                Some(r"c:\docker\resources\com.docker.backend.exe"),
            ),
            class(10, false),
        )
        .add(
            proc(11, "conhost.exe", Some(r"c:\windows\system32\conhost.exe")),
            class(10, false),
        );
        let groups = w.groups();
        assert_eq!(groups.len(), 1);
        let g = &groups[r"c:\docker\docker desktop.exe"];
        assert_eq!(g.kind, GroupKind::App);
        // 主进程在前，其余按名称。
        assert_eq!(pids(g), vec![10, 12, 11]);
    }

    #[test]
    fn the_same_program_started_twice_is_one_group() {
        let mut w = World::new();
        w.add(
            proc(10, "app.exe", Some(r"c:\app\app.exe")),
            class(10, true),
        )
        .add(
            proc(20, "app.exe", Some(r"c:\app\app.exe")),
            class(20, true),
        )
        .add(
            proc(21, "helper.exe", Some(r"c:\app\helper.exe")),
            class(20, false),
        );
        let groups = w.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(pids(&groups[r"c:\app\app.exe"]), vec![10, 20, 21]);
    }

    #[test]
    fn idle_programs_without_windows_are_not_listed() {
        let mut w = World::new();
        w.add(
            proc(10, "tool.exe", Some(r"c:\tool\tool.exe")),
            class(10, false),
        );
        assert!(w.groups().is_empty());
        let remembered = running_groups(&w.snapshot, &w.classes, &|p| p == r"c:\tool\tool.exe");
        assert_eq!(remembered.len(), 1);
    }

    #[test]
    fn excluded_programs_keep_their_own_group() {
        let mut w = World::new();
        let mut c = class(40, true);
        c.schedule = ScheduleScope::Excluded;
        w.add(proc(40, "ime.exe", Some(r"c:\ime\ime.exe")), c);
        assert_eq!(w.groups()[r"c:\ime\ime.exe"].kind, GroupKind::Excluded);
    }
}
