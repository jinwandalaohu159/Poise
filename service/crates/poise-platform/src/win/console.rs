//! 控制台模式下捕获 Ctrl+C / 关闭窗口，让服务能正常还原后退出。

use std::sync::OnceLock;

use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

type Stop = Box<dyn Fn() + Send + Sync>;

static STOP: OnceLock<Stop> = OnceLock::new();

unsafe extern "system" fn handler(_ctrl_type: u32) -> BOOL {
    if let Some(stop) = STOP.get() {
        stop();
    }
    true.into()
}

/// 收到 Ctrl+C、关闭窗口等信号时调用 `stop`（应只发送通知）。
pub fn on_stop(stop: impl Fn() + Send + Sync + 'static) -> windows::core::Result<()> {
    let _ = STOP.set(Box::new(stop));
    // SAFETY: 处理函数是静态函数，只发送通道消息。
    unsafe { SetConsoleCtrlHandler(Some(handler), true) }
}
