//! 角色：进程是什么（关键进程、受保护、反作弊、服务……），只靠系统属性判断；以及服务能否安全重启。

use std::collections::HashMap;

use super::{ClassifyInput, Role};
use crate::known;
use crate::model::{Pid, ProcessSample, Protection, Snapshot};

pub(super) fn detect_anti_cheat(snap: &Snapshot) -> bool {
    snap.processes.iter().any(|p| {
        known::is_anti_cheat_process(&p.facts.name)
            || p.facts
                .services
                .iter()
                .any(|s| known::is_anti_cheat_service(&s.name))
    })
}

pub(super) fn role_of(
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
