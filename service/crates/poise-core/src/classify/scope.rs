//! 调度范围与类别：哪些进程参与调度，以及用户为它选的类别。

use std::collections::HashMap;

use super::{Role, ScheduleScope};
use crate::known;
use crate::model::{Pid, ProcessSample};
use crate::prefs::{AppPreference, Preferences};

/// 程序自己没有设置偏好时，沿用所在进程树主程序的偏好（辅助进程跟随主程序）。自己就是一棵树的根
/// （独立出来的，例如标为游戏的启动器打开的全屏游戏本体）时，启动它的程序标为"游戏"就也算游戏：
/// 否则游戏本体一独立出来就丢了标记。
pub(super) fn preference_of(
    p: &ProcessSample,
    root_pid: Pid,
    by_pid: &HashMap<Pid, &ProcessSample>,
    prefs: &Preferences,
) -> AppPreference {
    match prefs.lookup(&p.facts) {
        AppPreference::Auto if root_pid != p.facts.key.pid => by_pid
            .get(&root_pid)
            .map_or(AppPreference::Auto, |root| prefs.lookup(&root.facts)),
        AppPreference::Auto => p
            .facts
            .parent_pid
            .and_then(|pp| by_pid.get(&pp))
            .filter(|parent| parent.is_parent_of(p))
            .map(|parent| prefs.lookup(&parent.facts))
            .filter(|pref| *pref == AppPreference::Game)
            .unwrap_or(AppPreference::Auto),
        own => own,
    }
}

/// Windows 自己的维护程序，或装在 Program Files 里的后台更新程序（见 `known`）。
fn maintenance(p: &ProcessSample) -> bool {
    (p.facts.in_windows_dir && known::is_maintenance_worker(&p.facts.name))
        || p.facts
            .image_path
            .as_deref()
            .is_some_and(known::is_maintenance_path)
}

/// 只调度用户会话里的普通程序。系统的东西交给系统：服务、会话 0 的进程、Windows 目录下的程序
/// （桌面外壳、输入法、系统界面）一律不碰；名单里的程序（输入、驱动、安全软件，`listed`）也不碰。
/// 唯一的例外是 Windows 自己的几个维护程序（[`ScheduleScope::Maintenance`]），只允许挪开。
pub(super) fn schedule_scope(p: &ProcessSample, role: &Role, listed: bool) -> ScheduleScope {
    match role {
        Role::UserApp if !p.facts.in_windows_dir && !listed => ScheduleScope::App,
        Role::UserApp | Role::ServiceHost | Role::ServiceChild { .. } | Role::SystemOther
            if maintenance(p) =>
        {
            ScheduleScope::Maintenance
        }
        _ => ScheduleScope::Excluded,
    }
}
