//! 进程句柄：自动关闭，并把 Win32 错误码映射成业务错误。

use poise_core::action::ExecError;
use poise_core::model::Pid;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, HANDLE,
    WIN32_ERROR,
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

/// 以最小权限打开进程。进程号不存在（已退出）时系统报"参数错误"，按"已退出"处理。
/// 要调整的进程用 [`crate::win::process::Target`]，它还会核对身份。
pub fn open_process(pid: Pid, access: PROCESS_ACCESS_RIGHTS) -> Result<OwnedHandle, ExecError> {
    // SAFETY: 只传入值类型参数，返回的句柄由 OwnedHandle 管理。
    unsafe { OpenProcess(access, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| {
            if WIN32_ERROR::from_error(&e) == Some(ERROR_INVALID_PARAMETER) {
                ExecError::Gone
            } else {
                map_error(&e)
            }
        })
}

/// 系统调用的错误：拒绝访问（Win32 错误码或内核状态码）单独区分，其余都是失败。
/// "参数错误"不代表进程已退出（例如系统不认识某个标志），要不要按退出处理由调用方核实。
pub fn map_error(e: &windows::core::Error) -> ExecError {
    /// 内核状态码 STATUS_ACCESS_DENIED 包装成的 HRESULT（带 FACILITY_NT_BIT）。
    const NT_ACCESS_DENIED: i32 = (0xC000_0022_u32 | 0x1000_0000) as i32;
    let denied =
        WIN32_ERROR::from_error(e) == Some(ERROR_ACCESS_DENIED) || e.code().0 == NT_ACCESS_DENIED;
    if denied {
        ExecError::AccessDenied
    } else {
        ExecError::Failed(e.message())
    }
}

/// 进程的完整路径（需要 PROCESS_QUERY_LIMITED_INFORMATION）。一般的路径放在栈上的缓冲区里，
/// 放不下（长路径）时再用最长路径大小的缓冲区读一次。
pub fn image_path_of(h: &OwnedHandle) -> Option<String> {
    /// 长路径的上限（字符）。
    const LONGEST: usize = 32_768;
    let read = |buf: &mut [u16]| -> windows::core::Result<String> {
        let mut len = buf.len() as u32;
        // SAFETY: 缓冲区长度与 len 一致。
        unsafe {
            QueryFullProcessImageNameW(
                h.raw(),
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
        }?;
        Ok(String::from_utf16_lossy(&buf[..len as usize]))
    };
    match read(&mut [0u16; 520]) {
        Ok(path) => Some(path),
        Err(e) if WIN32_ERROR::from_error(&e) == Some(ERROR_INSUFFICIENT_BUFFER) => {
            read(&mut vec![0u16; LONGEST]).ok()
        }
        Err(_) => None,
    }
}

/// 按进程 ID 取完整路径；进程已退出或无权查询时为 None。
pub fn image_path(pid: Pid) -> Option<String> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    image_path_of(&h)
}
