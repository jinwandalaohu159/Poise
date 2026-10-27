//! 进程句柄：自动关闭，并把 Win32 错误码映射成业务错误。

use poise_core::action::ExecError;
use poise_core::model::Pid;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE, WIN32_ERROR,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

/// 拥有所有权的句柄，离开作用域时关闭。
pub struct OwnedHandle(HANDLE);

// SAFETY: 内核对象句柄可以在任意线程使用和关闭。
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// 接管一个已经打开的句柄。
    pub fn from_raw(h: HANDLE) -> Self {
        Self(h)
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: 句柄由本对象独占，只关闭一次。
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// 以最小权限打开进程。
pub fn open_process(pid: Pid, access: PROCESS_ACCESS_RIGHTS) -> Result<OwnedHandle, ExecError> {
    // SAFETY: 只传入值类型参数，返回的句柄由 OwnedHandle 管理。
    unsafe { OpenProcess(access, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| map_error(&e))
}

pub fn map_error(e: &windows::core::Error) -> ExecError {
    let code = WIN32_ERROR::from_error(e).unwrap_or_default();
    if code == ERROR_ACCESS_DENIED {
        ExecError::AccessDenied
    } else if code == ERROR_INVALID_PARAMETER {
        ExecError::Gone
    } else {
        ExecError::Failed(e.message())
    }
}

/// 进程的完整路径（需要 PROCESS_QUERY_LIMITED_INFORMATION）。
pub fn image_path_of(h: &OwnedHandle) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: 缓冲区长度与 len 一致。
    unsafe {
        QueryFullProcessImageNameW(
            h.raw(),
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// 按进程 ID 取完整路径；进程已退出或无权查询时为 None。
pub fn image_path(pid: Pid) -> Option<String> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    image_path_of(&h)
}
