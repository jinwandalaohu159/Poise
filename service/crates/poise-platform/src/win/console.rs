//! 控制台模式下捕获 Ctrl+C / 关闭窗口，让服务能正常还原后退出。

use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

static STOP: OnceLock<Sender<()>> = OnceLock::new();

unsafe extern "system" fn handler(_ctrl_type: u32) -> BOOL {
    if let Some(tx) = STOP.get() {
        let _ = tx.send(());
    }
    true.into()
}

/// 收到 Ctrl+C、关闭窗口等信号时向 `tx` 发送一次停止通知。
pub fn on_stop(tx: Sender<()>) -> windows::core::Result<()> {
    let _ = STOP.set(tx);
    // SAFETY: 处理函数是静态函数，只发送通道消息。
    unsafe { SetConsoleCtrlHandler(Some(handler), true) }
}
