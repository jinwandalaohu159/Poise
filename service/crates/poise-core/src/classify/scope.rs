//! 调度范围与类别：哪些进程参与调度，以及用户为它选的类别。

use std::collections::HashMap;

use super::{Role, ScheduleScope};
use crate::known;
use crate::model::{Pid, ProcessSample};
use crate::prefs::{AppPreference, Preferences};

/// 程序自己没有设置偏好时，沿用所在进程树主程序的偏好（辅助进程跟随主程序）。
pub(super) fn preference_of(
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

/// 只调度用户会话里的普通程序。系统的东西交给系统：服务、会话 0 的进程、Windows 目录下的程序
/// （桌面外壳、输入法、系统界面）一律不碰；名单里的程序（输入、驱动、安全软件）也不碰。
pub(super) fn schedule_scope(p: &ProcessSample, role: &Role) -> ScheduleScope {
    let name = p.facts.name.as_str();
    match role {
        Role::UserApp
            if !p.facts.in_windows_dir
                && !known::is_never_scheduled(name)
                && !p
                    .facts
                    .image_path
                    .as_deref()
                    .is_some_and(known::is_never_scheduled_path) =>
        {
            ScheduleScope::App
        }
        _ => ScheduleScope::Excluded,
    }
}
