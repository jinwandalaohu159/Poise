//! 界面程序（Poise.exe，WinUI）：按需启动；已经在运行时用命名事件通知它，不再启动第二个。

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use poise_core::model::Pid;
use poise_platform::win::handle::image_path;
use poise_platform::win::wide::WideString;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use crate::session::desktop;

/// 与界面约定的事件名（界面在启动时创建）。
const EVENT_FLYOUT: &str = r"Local\Poise.App.Flyout";
const EVENT_ACTIVATE: &str = r"Local\Poise.App.Activate";
const EVENT_EXIT: &str = r"Local\Poise.App.Exit";
const EVENT_DISMISS: &str = r"Local\Poise.App.Dismiss";

/// 等界面交出前台的最长时间。
const YIELD_TIMEOUT: Duration = Duration::from_millis(300);
const YIELD_POLL: Duration = Duration::from_millis(15);

/// 界面程序与托盘代理放在同一目录。开发时可用环境变量 POISE_UI_EXE 指定。
pub fn exe_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("POISE_UI_EXE") {
        return Some(PathBuf::from(p));
    }
    Some(std::env::current_exe().ok()?.parent()?.join("Poise.exe"))
}

/// 该进程是不是界面程序。
pub fn is_ui_process(pid: Pid) -> bool {
    image_path(pid)
        .zip(exe_path())
        .is_some_and(|(path, ui)| path.eq_ignore_ascii_case(&ui.to_string_lossy()))
}

/// 开关托盘面板。
pub fn toggle_flyout() {
    if !signal(EVENT_FLYOUT) {
        launch(&["--flyout"]);
    }
}

/// 打开主窗口。
pub fn open_window() {
    if !signal(EVENT_ACTIVATE) {
        launch(&[]);
    }
}

/// 让界面退出（托盘菜单"退出"）。
pub fn exit() {
    signal(EVENT_EXIT);
}

/// 弹出托盘菜单前调用：界面在前台时，别的进程拿不到前台，菜单就无法在点别处时收起。
/// 请界面收起托盘面板并交出前台，最多等一小会儿。
pub fn reclaim_foreground() {
    let ui_in_front = |hwnd| desktop::pid_of(hwnd).is_some_and(is_ui_process);
    let front = desktop::foreground_window();
    if !ui_in_front(front) || !signal(EVENT_DISMISS) {
        return;
    }
    let deadline = Instant::now() + YIELD_TIMEOUT;
    while Instant::now() < deadline && desktop::foreground_window() == front {
        std::thread::sleep(YIELD_POLL);
    }
}

fn signal(name: &str) -> bool {
    allow_foreground();
    let name = WideString::new(name);
    // SAFETY: 名称以 NUL 结尾；句柄用完即关。
    unsafe {
        let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, name.as_pcwstr()) else {
            return false;
        };
        let ok = SetEvent(event).is_ok();
        let _ = CloseHandle(event);
        ok
    }
}

/// 用户刚点了托盘图标，前台权限在托盘代理手里；转交给界面，让面板和窗口能出现在最前面。
fn allow_foreground() {
    // SAFETY: 无副作用的权限转交。
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

fn launch(args: &[&str]) {
    allow_foreground();
    let Some(exe) = exe_path() else { return };
    if let Err(e) = Command::new(&exe).args(args).spawn() {
        log::warn!("启动界面失败（{}）：{e}", exe.display());
    }
}
