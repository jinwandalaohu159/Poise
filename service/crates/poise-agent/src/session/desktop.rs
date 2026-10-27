//! 桌面上的状态：前台窗口、全屏、可见窗口、桌面外壳、输入空闲时间。只读。

use std::collections::HashSet;

use poise_core::model::Pid;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, WS_CAPTION,
};
use windows::core::BOOL;

pub fn pid_of(hwnd: HWND) -> Option<Pid> {
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0;
    // SAFETY: 输出到本地变量。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

pub fn foreground_window() -> HWND {
    // SAFETY: 无参数。
    unsafe { GetForegroundWindow() }
}

pub fn shell_pid() -> Option<Pid> {
    // SAFETY: 无参数。
    pid_of(unsafe { GetShellWindow() })
}

/// 可见的顶层窗口所属的进程。为了安全，最小化和其他虚拟桌面上的窗口也算在内：
/// 有窗口的程序绝不会被清理。
pub fn visible_window_pids() -> Vec<Pid> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam 是下面传入的 HashSet 指针，枚举期间有效。
        let pids = unsafe { &mut *(lparam.0 as *mut HashSet<Pid>) };
        // SAFETY: 系统传入的窗口句柄。
        if unsafe { IsWindowVisible(hwnd) }.as_bool()
            && let Some(pid) = pid_of(hwnd)
        {
            pids.insert(pid);
        }
        true.into()
    }
    let mut pids = HashSet::new();
    // SAFETY: 回调只在 EnumWindows 返回前被调用。
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut pids as *mut _ as isize)) };
    pids.into_iter().collect()
}

/// 独占全屏、演示模式，或无边框窗口铺满整个显示器（多数游戏的"全屏"）。
pub fn is_fullscreen(foreground: HWND) -> bool {
    // SAFETY: 输出到本地变量。
    if let Ok(state) = unsafe { SHQueryUserNotificationState() }
        && matches!(
            state,
            QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE | QUNS_BUSY
        )
    {
        return true;
    }
    if foreground.is_invalid() || is_desktop(foreground) || has_caption(foreground) {
        return false;
    }
    let mut rect = RECT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: 输出到本地结构体。
    unsafe {
        if GetWindowRect(foreground, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }
    }
    let m = info.rcMonitor;
    rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
}

/// 有标题栏的窗口即使铺满屏幕（例如任务栏自动隐藏时最大化）也不算全屏。
fn has_caption(hwnd: HWND) -> bool {
    // SAFETY: 读取窗口样式。
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    style & WS_CAPTION.0 == WS_CAPTION.0
}

fn is_desktop(hwnd: HWND) -> bool {
    let mut name = [0u16; 64];
    // SAFETY: 缓冲区长度与传入一致。
    let len = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
    let class = String::from_utf16_lossy(&name[..len]);
    matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd")
}

/// 距离最后一次键盘或鼠标输入的毫秒数。
pub fn idle_ms() -> u64 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: 输出到本地结构体。
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return 0;
    }
    // GetTickCount 与 dwTime 都是 32 位毫秒计数，按回绕相减。
    // SAFETY: 无参数。
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    u64::from(now.wrapping_sub(info.dwTime))
}
