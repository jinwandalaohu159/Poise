//! 更新之后，把各个用户会话里的托盘代理重新启动。安装程序先停服务、再关掉所有会话里的托盘代理和界面；
//! 更新助手只能重新启动它自己会话里的那个，同时登录着的其他用户的托盘代理就没了，Poise 对他们也就停了。
//! 服务停止时记下哪些会话里连着托盘代理；下次启动时，同一次开机里、会话里还是同一个用户的，在那个会话里
//! 以那个用户的身份把它重新启动。已经在运行的（服务只是重启了一下）由托盘代理自己的单实例检查挡住。
//! 用户自己退出了 Poise 的会话没有连着托盘代理，不会被记下。

use std::path::Path;

use poise_platform::act::launcher::{Launcher, UserSessionLauncher};
use poise_platform::observe::process::cmdline::LaunchSpec;
use poise_platform::observe::system;
use poise_platform::persist::storage::JsonFile;
use poise_platform::settings::user::UserContext;
use serde::{Deserialize, Serialize};

use super::Engine;
use super::growth::boot_unix_s;
use crate::config::RunMode;

const FILE: &str = "agents.json";
const AGENT_EXE: &str = "poise-agent.exe";

/// 开机时刻由"现在 - 开机时长"推算，前后两次相差一两秒：差这么多以内算同一次开机。
const SAME_BOOT_SLACK_S: u64 = 60;

/// 停止时连着托盘代理的会话。
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Record {
    boot_unix_s: u64,
    sessions: Vec<AgentSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AgentSession {
    id: u32,
    /// 会话里的用户：会话编号会被重新使用，换了用户不替他启动。
    sid: String,
}

impl Engine {
    /// 服务停止时（不是关机）记下连着托盘代理的会话。
    pub(super) fn remember_agents(&self) {
        if self.cfg.mode != RunMode::Service {
            return;
        }
        let Some(view) = &self.view else {
            return;
        };
        let mut sessions: Vec<AgentSession> = Vec::new();
        for pid in self.clients.pids() {
            let Some(p) = view.snapshot.find(pid) else {
                continue;
            };
            let id = p.facts.session_id;
            if !p.facts.name.eq_ignore_ascii_case(AGENT_EXE) || sessions.iter().any(|s| s.id == id)
            {
                continue;
            }
            if let Some(sid) = UserContext::for_session(id).and_then(|u| u.sid) {
                sessions.push(AgentSession { id, sid });
            }
        }
        let record = Record {
            boot_unix_s: boot_unix_s(system::uptime_ms()),
            sessions,
        };
        if let Err(e) = JsonFile::new(self.cfg.data_dir.join(FILE)).save(&record) {
            log::warn!("记下托盘代理所在的会话失败：{e}");
        }
    }
}

/// 服务启动时：在上次停止时连着托盘代理的会话里把它重新启动（见模块说明）。记录用一次就删。
pub fn relaunch_agents(data_dir: &Path) {
    let file = JsonFile::<Record>::new(data_dir.join(FILE));
    let Some(record) = file.load() else {
        return;
    };
    let _ = std::fs::remove_file(file.path());
    let boot = boot_unix_s(system::uptime_ms());
    if record.boot_unix_s.abs_diff(boot) > SAME_BOOT_SLACK_S || record.sessions.is_empty() {
        return;
    }
    let Some(spec) = agent_spec() else {
        return;
    };
    for s in record.sessions {
        let same_user = UserContext::for_session(s.id)
            .and_then(|u| u.sid)
            .is_some_and(|sid| sid.eq_ignore_ascii_case(&s.sid));
        if !same_user {
            continue;
        }
        match UserSessionLauncher.launch(&spec, s.id) {
            Ok(pid) => log::info!("在会话 {} 里重新启动了托盘代理（进程 {pid}）", s.id),
            Err(e) => log::warn!("在会话 {} 里重新启动托盘代理失败：{e:?}", s.id),
        }
    }
}

/// 和服务在同一目录（安装目录）里的托盘代理。
fn agent_spec() -> Option<LaunchSpec> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let exe = dir.join(AGENT_EXE);
    if !exe.is_file() {
        return None;
    }
    let image_path = exe.to_string_lossy().into_owned();
    Some(LaunchSpec {
        command_line: format!("\"{image_path}\""),
        image_path,
        working_dir: dir.to_string_lossy().into_owned(),
    })
}
