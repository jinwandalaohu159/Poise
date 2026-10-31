//! 服务管理器的薄封装：句柄自动关闭，打开管理器与服务，数正在运行的依赖服务。
//! 观测（服务表）和动作（重启服务）共用。

use windows::Win32::System::Services::{
    CloseServiceHandle, EnumDependentServicesW, OpenSCManagerW, OpenServiceW, SC_HANDLE,
    SERVICE_ACTIVE, SERVICE_ENUMERATE_DEPENDENTS,
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
