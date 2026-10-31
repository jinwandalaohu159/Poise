//! Windows 服务表：哪个进程承载了哪些服务（只读观测）。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use poise_core::model::{Pid, ProcessKey, ServiceFacts};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SHUTDOWN_IN_PROGRESS};
use windows::Win32::System::Services::{
    ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, SC_ENUM_PROCESS_INFO, SC_MANAGER_CONNECT,
    SC_MANAGER_ENUMERATE_SERVICE, SERVICE_ACCEPT_STOP, SERVICE_ACTIVE, SERVICE_WIN32,
};
use windows::core::PCWSTR;

use crate::win::scm::{open_manager, running_dependents};

/// 进程 → 服务 的映射。
///
/// 枚举一次要对服务管理器做几十毫秒的远程调用，所以不按固定间隔刷新，而是看服务管理器拉起的进程：
/// 有服务启动（新进程）或退出（进程没了）就立即重新枚举，比按间隔刷新更及时。服务也可能在已有的
/// 共享宿主进程里启动，看不出进程变化，所以另外隔一段时间兜底刷新一次。
///
/// 每个独占进程的服务还要数一下依赖它的服务（能不能安全重启）：一百多个服务逐个问一遍要四五十毫秒，
/// 所以每个服务的结果记一会儿（见 [`ServiceMap::DEPENDENTS_FOR`]），重启前另外再核对一次。
#[derive(Default)]
pub struct ServiceMap {
    by_pid: HashMap<Pid, Vec<ServiceFacts>>,
    refreshed_at: Option<Instant>,
    /// 上次枚举时服务管理器拉起的进程。
    hosts: Option<HashSet<ProcessKey>>,
    /// 各服务正在运行的依赖服务有几个，以及什么时候数的。
    dependents: HashMap<String, (u32, Instant)>,
}

impl ServiceMap {
    /// 兜底刷新的间隔；找不到服务管理器进程时（看不到服务进程的变化）按短间隔刷新。
    const BACKSTOP_EVERY: Duration = Duration::from_secs(120);
    const BLIND_EVERY: Duration = Duration::from_secs(30);

    /// `hosts`：服务管理器拉起的进程；找不到服务管理器时为 None。
    /// 依赖服务的数目记多久。
    const DEPENDENTS_FOR: Duration = Duration::from_secs(10 * 60);

    pub fn refresh(&mut self, hosts: Option<HashSet<ProcessKey>>) {
        let every = if hosts.is_some() {
            Self::BACKSTOP_EVERY
        } else {
            Self::BLIND_EVERY
        };
        let stale = self.refreshed_at.is_none_or(|t| t.elapsed() >= every);
        let changed = hosts.is_some() && hosts != self.hosts;
        if stale || changed {
            match enumerate(&mut self.dependents) {
                Ok(m) => self.by_pid = m,
                // 关机时服务管理器不再回答：不是故障。
                Err(e) if e.code() == ERROR_SHUTDOWN_IN_PROGRESS.to_hresult() => {
                    log::debug!("枚举服务失败：{e}")
                }
                Err(e) => log::warn!("枚举服务失败：{e}"),
            }
            self.refreshed_at = Some(Instant::now());
            self.hosts = hosts;
        }
    }

    pub fn services_of(&self, pid: Pid) -> Vec<ServiceFacts> {
        self.by_pid.get(&pid).cloned().unwrap_or_default()
    }
}

fn enumerate(
    dependents: &mut HashMap<String, (u32, Instant)>,
) -> windows::core::Result<HashMap<Pid, Vec<ServiceFacts>>> {
    let scm = open_manager(SC_MANAGER_ENUMERATE_SERVICE | SC_MANAGER_CONNECT)?;
    // 按 8 字节对齐分配（结构体里有指针）；放不下时（远程桌面服务器上每个用户都有一套服务）按系统
    // 说的大小加大再枚举一遍。
    let mut buf = vec![0u64; 256 * 1024 / 8];
    let mut count = 0u32;
    let mut attempts = 0;
    loop {
        attempts += 1;
        let mut needed = 0u32;
        // SAFETY: 把 u64 缓冲区按字节交给系统，长度一致。
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
        // SAFETY: 缓冲区由我们分配，系统写入不超过其长度。
        let result = unsafe {
            EnumServicesStatusExW(
                scm.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_ACTIVE,
                Some(bytes),
                &mut needed,
                &mut count,
                None,
                PCWSTR::null(),
            )
        };
        match result {
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() && attempts < 3 => {
                let more = (needed as usize).div_ceil(8) + 1024;
                buf.resize(buf.len() + more, 0);
            }
            other => {
                other?;
                break;
            }
        }
    }
    // SAFETY: 系统在缓冲区开头写入 count 个结构（缓冲区按 8 字节对齐）。
    let entries = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
            count as usize,
        )
    };
    let mut map: HashMap<Pid, Vec<ServiceFacts>> = HashMap::new();
    for e in entries {
        let status = e.ServiceStatusProcess;
        if status.dwProcessId == 0 {
            continue;
        }
        // SAFETY: 名称指针指向缓冲区内的 NUL 结尾字符串。
        let name = unsafe { e.lpServiceName.to_string() }.unwrap_or_default();
        // SAFETY: 同上。
        let display_name = unsafe { e.lpDisplayName.to_string() }.unwrap_or_default();
        map.entry(status.dwProcessId)
            .or_default()
            .push(ServiceFacts {
                name,
                display_name,
                accepts_stop: status.dwControlsAccepted & SERVICE_ACCEPT_STOP != 0,
                running_dependents: 0,
            });
    }
    // 只有单服务进程才可能被重启，只为它们计算依赖者；最近数过的直接用。
    dependents.retain(|_, (_, at)| at.elapsed() < ServiceMap::DEPENDENTS_FOR);
    for services in map.values_mut() {
        if let [only] = services.as_mut_slice() {
            only.running_dependents = match dependents.get(&only.name) {
                Some((n, _)) => *n,
                None => {
                    let n = running_dependents(&scm, &only.name).unwrap_or(u32::MAX);
                    dependents.insert(only.name.clone(), (n, Instant::now()));
                    n
                }
            };
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_enumerates_only_when_the_service_processes_change() {
        let key = |pid| ProcessKey { pid, created: 1 };
        let mut m = ServiceMap::default();
        m.refresh(Some(HashSet::from([key(1)])));
        let first = m.refreshed_at;
        m.refresh(Some(HashSet::from([key(1)])));
        assert_eq!(m.refreshed_at, first);
        m.refresh(Some(HashSet::from([key(1), key(2)])));
        assert_ne!(m.refreshed_at, first);
    }

    #[test]
    fn finds_services_hosted_in_svchost() {
        let mut m = ServiceMap::default();
        m.refresh(None);
        assert!(!m.by_pid.is_empty());
        assert!(
            m.by_pid
                .values()
                .flatten()
                .any(|s| s.name.eq_ignore_ascii_case("Audiosrv")
                    || s.name.eq_ignore_ascii_case("EventLog"))
        );
    }
}
