//! COM 初始化。

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// 当前线程加入多线程套间（重复调用无害）。
pub fn ensure_initialized() {
    // S_FALSE 表示已经初始化过；RPC_E_CHANGED_MODE 表示线程已是单线程套间，此时照常可用。
    // SAFETY: 只影响当前线程的 COM 状态。
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}
