//! 角色：进程是什么（关键进程、受保护、反作弊、服务……），只靠系统属性判断；以及服务能否安全重启。

use super::{ClassifyInput, Role};
use crate::fast_hash::{HashMap, HashSet};
use crate::known;
use crate::model::{Pid, ProcessFacts, ProcessSample, Protection, Snapshot};

/// 反作弊程序：名单里的进程和服务，装在已知反作弊目录里的程序，以及与名单里的程序装在同一目录的
/// 所有程序——同一套反作弊往往有好几个程序（例如 Vanguard 的 vgc、vgm、vgtray），名单不可能列全。
#[derive(Debug, Default)]
pub(super) struct AntiCheat {
    /// 正在运行的已知反作弊程序所在的目录（小写，不带末尾的分隔符）。
    dirs: HashSet<String>,
    present: bool,
}

impl AntiCheat {
    pub(super) fn detect(snap: &Snapshot) -> Self {
        let mut found = Self::default();
        for p in snap.processes.iter().filter(|p| listed(&p.facts)) {
            found.present = true;
            if let Some(dir) = p.facts.image_path.as_deref().and_then(dir_of) {
                found.dirs.insert(dir.to_string());
            }
        }
        found
    }

    /// 有反作弊程序在运行。
    pub(super) fn present(&self) -> bool {
        self.present
    }

    pub(super) fn covers(&self, f: &ProcessFacts) -> bool {
        listed(f)
            || f.image_path.as_deref().is_some_and(|path| {
                known::is_anti_cheat_path(path)
                    || dir_of(path).is_some_and(|d| self.dirs.contains(d))
            })
    }
}

fn listed(f: &ProcessFacts) -> bool {
    known::is_anti_cheat_process(&f.name)
        || f.services
            .iter()
            .any(|s| known::is_anti_cheat_service(&s.name))
}

fn dir_of(path: &str) -> Option<&str> {
    path.rsplit_once('\\').map(|(dir, _)| dir)
}

pub(super) fn role_of(
    p: &ProcessSample,
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    anti_cheat: &AntiCheat,
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
    if anti_cheat.covers(f) {
        return Role::AntiCheat;
    }
    if known::is_vm_process(&f.name) {
        return Role::VirtualMachine;
    }
    if f.name == "dwm.exe" && f.session_id != 0 && f.in_windows_dir {
        return Role::Compositor;
    }
    if !f.services.is_empty() {
        return Role::ServiceHost;
    }
    if f.session_id == 0
        && f.in_windows_dir
        && let Some(parent) = f.parent_pid.and_then(|pp| by_pid.get(&pp))
        && parent.is_parent_of(p)
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

pub(super) fn restartable_service(
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
