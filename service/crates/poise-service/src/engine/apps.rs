//! 程序列表：正在运行的用户程序（含其全部进程）；系统一栏（后台杂活、不参与调度的程序）；
//! 未运行的程序（设置过类别的，以及以前运行过的）。运行过的程序保存在本机。
//! 分组规则见 [`poise_core::app_groups`]。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use poise_core::app_groups::{self, AppGroup, GroupKind};
use poise_core::classify::Classification;
use poise_core::model::ProcessSample;
use poise_core::prefs::AppPreference;
use poise_protocol::{AppEntry, AppKind, AppProcess};

use super::{Engine, View, convert, unix_now_ms};

/// 运行过的程序保存在数据目录的这个文件里。
pub(super) const SEEN_FILE: &str = "seen_apps.json";

impl Engine {
    /// 程序偏好页的列表：正在运行的（含系统一栏）加上未运行的，按名称排序。界面按 `kind` 和 `running` 分栏。
    pub fn apps(&self) -> Vec<AppEntry> {
        let Some(view) = &self.view else {
            return self.idle_entries(&HashSet::new());
        };
        let mut apps = self.running_entries(view);
        // 作为别的程序的一部分在运行（归到主程序名下）的程序，也不算"未运行"。
        let running: HashSet<&str> = view
            .snapshot
            .processes
            .iter()
            .filter_map(|p| p.facts.image_path.as_deref())
            .collect();
        apps.extend(self.idle_entries(&running));
        apps.sort_by_cached_key(|a| a.name.to_lowercase());
        apps
    }

    fn running_entries(&self, view: &View) -> Vec<AppEntry> {
        self.running_groups(view)
            .into_values()
            .map(|g| {
                let kind = convert::app_kind(g.kind);
                let separable = g.separable_members();
                let processes: Vec<AppProcess> = g
                    .members
                    .iter()
                    .map(|&(p, c)| {
                        let preference = separable
                            .contains(&p.facts.key.pid)
                            .then(|| self.settings.preferences.lookup(&p.facts));
                        self.app_process(view, p, c, preference)
                    })
                    .collect();
                AppEntry {
                    pid: Some(g.main.facts.key.pid),
                    name: g.main.facts.friendly_name(),
                    path: g.path.to_string(),
                    preference: match kind {
                        AppKind::App => self.settings.preferences.lookup(&g.main.facts),
                        AppKind::Excluded => AppPreference::Auto,
                    },
                    running: true,
                    is_game: g.is_game,
                    schedule: Some(self.app_schedule(g.main, g.class)),
                    kind,
                    cpu_pct: processes.iter().map(|p| p.cpu_pct).sum(),
                    memory_bytes: processes.iter().map(|p| p.memory_bytes).sum(),
                    processes,
                }
            })
            .collect()
    }

    fn app_process(
        &self,
        view: &View,
        p: &ProcessSample,
        c: &Classification,
        preference: Option<AppPreference>,
    ) -> AppProcess {
        let cpus = view.snapshot.system.logical_cpus;
        AppProcess {
            pid: p.facts.key.pid,
            name: p.facts.name.clone(),
            path: p.facts.image_path.clone().unwrap_or_default(),
            preference,
            cpu_pct: self.activity.usage(&p.facts.key).share_pct(cpus),
            memory_bytes: p.metrics.memory_bytes,
            schedule: self.app_schedule(p, c),
        }
    }

    fn running_groups<'a>(&self, view: &'a View) -> BTreeMap<String, AppGroup<'a>> {
        app_groups::running_groups(&view.snapshot, &view.classes, &|path| self.remembered(path))
    }

    /// 没在运行的：设置过类别的，以及以前运行过的。
    fn idle_entries(&self, running: &HashSet<&str>) -> Vec<AppEntry> {
        let prefs = &self.settings.preferences.apps;
        let configured = prefs
            .iter()
            .filter(|(_, p)| **p != AppPreference::Auto)
            .map(|(path, _)| path);
        let paths: BTreeSet<&String> = configured
            .chain(self.seen.apps.keys())
            .filter(|path| !running.contains(path.as_str()))
            .collect();
        paths
            .into_iter()
            .map(|path| {
                let preference = prefs.get(path).copied().unwrap_or_default();
                AppEntry {
                    pid: None,
                    name: self
                        .seen
                        .apps
                        .get(path)
                        .map_or_else(|| file_stem(path), |s| s.name.clone()),
                    path: path.clone(),
                    preference,
                    running: false,
                    is_game: preference == AppPreference::Game,
                    schedule: None,
                    kind: AppKind::App,
                    cpu_pct: 0.0,
                    memory_bytes: 0,
                    processes: Vec::new(),
                }
            })
            .collect()
    }

    /// 每个采样周期调用：记住正在运行、有窗口的用户程序（命令行工具、后台小程序不记），有变化时保存。
    pub(super) fn remember_running_apps(&mut self) {
        let Some(view) = &self.view else {
            return;
        };
        let running: Vec<(String, String)> = self
            .running_groups(view)
            .into_values()
            .filter(|g| g.kind == GroupKind::App && g.class.tree_has_window)
            .map(|g| (g.path.to_string(), g.main.facts.friendly_name()))
            .collect();
        let now_s = unix_now_ms() / 1000;
        let mut changed = false;
        for (path, name) in &running {
            changed |= self.seen.observe(path, name, now_s);
        }
        if changed {
            self.save_seen_apps();
        }
    }

    /// 启动时：去掉太久没运行、文件已不存在的程序。
    pub(super) fn prune_seen_apps(&mut self) {
        let now_s = unix_now_ms() / 1000;
        if self
            .seen
            .prune(now_s, |path| std::path::Path::new(path).exists())
        {
            self.save_seen_apps();
        }
    }

    /// 设置过类别或运行过的程序：只要在运行就列出（即使没有窗口、也没在干活）。
    fn remembered(&self, path: &str) -> bool {
        self.seen.apps.contains_key(path) || self.settings.preferences.apps.contains_key(path)
    }

    fn save_seen_apps(&self) {
        if let Err(e) = self.seen_file.save(&self.seen) {
            log::warn!("保存运行过的程序失败：{e}");
        }
    }
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}
