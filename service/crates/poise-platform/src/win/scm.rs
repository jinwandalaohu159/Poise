//! 服务管理器的薄封装：句柄自动关闭，打开管理器与服务，查询状态，数正在运行的依赖服务。
//! 观测（服务表）、动作（重启服务）和托盘代理（确认管道另一头是服务）共用。

use windows::Win32::System::Services::{
    CloseServiceHandle, EnumDependentServicesW, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_ACTIVE,
    SERVICE_ENUMERATE_DEPENDENTS, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS_PROCESS,
};
use windows::core::PCWSTR;

use crate::win::wide::WideString;

pub struct ScHandle(pub SC_HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本对象独占。
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

pub fn open_manager(access: u32) -> windows::core::Result<ScHandle> {
    // SAFETY: 本机、默认数据库。
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }.map(ScHandle)
}

pub fn open_service(scm: &ScHandle, name: &str, access: u32) -> windows::core::Result<ScHandle> {
    let w = WideString::new(name);
    // SAFETY: 名称以 NUL 结尾。
    unsafe { OpenServiceW(scm.0, w.as_pcwstr(), access) }.map(ScHandle)
}

pub fn status(svc: &ScHandle) -> windows::core::Result<SERVICE_STATUS_PROCESS> {
    let mut s = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    // SAFETY: 缓冲区即本地结构体。
    unsafe {
        QueryServiceStatusEx(
            svc.0,
            SC_STATUS_PROCESS_INFO,
            Some(std::slice::from_raw_parts_mut(
                &mut s as *mut _ as *mut u8,
                size_of::<SERVICE_STATUS_PROCESS>(),
            )),
            &mut needed,
        )
    }?;
    Ok(s)
}

/// 正在运行的服务的进程号；没有这个服务、没在运行或无权查询时为 None。
/// 查询状态普通用户也有权限（打开服务进程本身则没有）。
pub fn running_pid(name: &str) -> Option<u32> {
    let scm = open_manager(SC_MANAGER_CONNECT).ok()?;
    let svc = open_service(&scm, name, SERVICE_QUERY_STATUS).ok()?;
    let s = status(&svc).ok()?;
    (s.dwCurrentState == SERVICE_RUNNING && s.dwProcessId != 0).then_some(s.dwProcessId)
}

/// 依赖这个服务、正在运行的服务有几个（重启它会连带停掉它们）。
pub fn running_dependents(scm: &ScHandle, name: &str) -> windows::core::Result<u32> {
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
