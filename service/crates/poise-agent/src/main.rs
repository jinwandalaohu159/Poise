//! Poise 托盘代理：常驻用户会话（开机自启的就是它），占用极小。
//!
//! - 托盘图标与右键菜单；左键开关托盘面板（界面按需启动）；
//! - 感知前台窗口、全屏、可见窗口、音频，上报给服务（服务在会话 0 看不到用户桌面）；
//! - 定期检查 GitHub 上有没有新版本，有就提示。

#![windows_subsystem = "windows"]

mod app;
mod foreground;
mod link;
mod logging;
mod prefs;
mod session;
mod tray;
mod ui_app;
mod update;

use poise_platform::act::self_tune;
use poise_platform::observe::topology::CpuTopology;
use poise_platform::win::wide::WideString;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

/// 每个用户只运行一个托盘代理。界面用同一个名字判断托盘代理是否在运行。
const INSTANCE_MUTEX: &str = r"Local\Poise.Agent";

fn main() {
    let name = WideString::new(INSTANCE_MUTEX);
    // SAFETY: 互斥体在进程整个生命周期内持有（不关闭句柄）。
    let _instance = unsafe { CreateMutexW(None, true, name.as_pcwstr()) };
    // SAFETY: 读取上一个调用的错误码。
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }
    // SAFETY: 进程启动时设置一次。
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    logging::init();
    self_tune::apply(&CpuTopology::detect());
    // 前台切换的通知在这个线程（消息循环）里收到，要及时转给服务。
    self_tune::responsive_thread();
    log::info!("Poise 托盘代理启动");
    app::run();
    log::info!("Poise 托盘代理退出");
}
