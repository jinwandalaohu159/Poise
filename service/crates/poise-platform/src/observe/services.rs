//! Windows 服务表：哪个进程承载了哪些服务（只读观测）。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use poise_core::model::{Pid, ProcessKey, ServiceFacts};
use windows::Win32::System::Services::{
    CloseServiceHandle, ENUM_SERVICE_STATUS_PROCESSW, EnumDependentServicesW,
    EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, SC_ENUM_PROCESS_INFO, SC_HANDLE,
    SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_ACCEPT_STOP, SERVICE_ACTIVE,
    SERVICE_ENUMERATE_DEPENDENTS, SERVICE_WIN32,
};
use windows::core::PCWSTR;

use crate::win::wide::WideString;

pub(crate) struct ScHandle(pub(crate) SC_HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本对象独占。
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

pub(crate) fn open_manager(access: u32) -> windows::core::Result<ScHandle> {
    // SAFETY: 本机、默认数据库。
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }.map(ScHandle)
}

pub(crate) fn open_service(
    scm: &ScHandle,
    name: &str,
    access: u32,
) -> windows::core::Result<ScHandle> {
    let w = WideString::new(name);
    // SAFETY: 名称以 NUL 结尾。
    unsafe { OpenServiceW(scm.0, w.as_pcwstr(), access) }.map(ScHandle)
}

/// 进程 → 服务 的映射。
///
/// 枚举一次要对服务管理器做几十毫秒的远程调用，所以不按固定间隔刷新，而是看服务管理器拉起的进程：
/// 有服务启动（新进程）或退出（进程没了）就立即重新枚举，比按间隔刷新更及时。服务也可能在已有的
/// 共享宿主进程里启动，看不出进程变化，所以另外隔一段时间兜底刷新一次。
#[derive(Default)]
pub struct ServiceMap {
    by_pid: HashMap<Pid, Vec<ServiceFacts>>,
    refreshed_at: Option<Instant>,
    /// 上次枚举时服务管理器拉起的进程。
    hosts: Option<HashSet<ProcessKey>>,
}

impl ServiceMap {
    /// 兜底刷新的间隔；找不到服务管理器进程时（看不到服务进程的变化）按短间隔刷新。
    const BACKSTOP_EVERY: Duration = Duration::from_secs(120);
    const BLIND_EVERY: Duration = Duration::from_secs(30);

    /// `hosts`：服务管理器拉起的进程；找不到服务管理器时为 None。
    pub fn refresh(&mut self, hosts: Option<HashSet<ProcessKey>>) {
        let every = if hosts.is_some() {
            Self::BACKSTOP_EVERY
        } else {
            Self::BLIND_EVERY
        };
        let stale = self.refreshed_at.is_none_or(|t| t.elapsed() >= every);
        let changed = hosts.is_some() && hosts != self.hosts;
        if stale || changed {
            match enumerate() {
                Ok(m) => self.by_pid = m,
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

fn enumerate() -> windows::core::Result<HashMap<Pid, Vec<ServiceFacts>>> {
    let scm = open_manager(SC_MANAGER_ENUMERATE_SERVICE | SC_MANAGER_CONNECT)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut needed = 0u32;
    let mut count = 0u32;
    // SAFETY: 缓冲区由我们分配，系统写入不超过其长度。
    unsafe {
        EnumServicesStatusExW(
            scm.0,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_ACTIVE,
            Some(&mut buf),
            &mut needed,
            &mut count,
            None,
            PCWSTR::null(),
        )
    }?;
    // SAFETY: 系统在缓冲区开头写入 count 个结构。
    let entries = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
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
    // 只有单服务进程才可能被重启，只为它们计算依赖者。
    for services in map.values_mut() {
        if let [only] = services.as_mut_slice() {
            only.running_dependents = running_dependents(&scm, &only.name).unwrap_or(u32::MAX);
        }
    }
    Ok(map)
}

fn running_dependents(scm: &ScHandle, name: &str) -> windows::core::Result<u32> {
    let svc = open_service(scm, name, SERVICE_ENUMERATE_DEPENDENTS)?;
    let mut needed = 0u32;
    let mut count = 0u32;
    // SAFETY: 先用空缓冲区询问大小。
    let first =
        unsafe { EnumDependentServicesW(svc.0, SERVICE_ACTIVE, None, 0, &mut needed, &mut count) };
    if first.is_ok() {
        return Ok(count);
    }
    let mut buf = vec![0u8; needed as usize];
    // SAFETY: 缓冲区大小为系统要求的字节数。
    unsafe {
        EnumDependentServicesW(
            svc.0,
            SERVICE_ACTIVE,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
            &mut count,
        )
    }?;
    Ok(count)
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
