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
    EnumChildWindows, EnumWindows, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetShellWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, WS_CAPTION,
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

/// 窗口真正属于哪个程序。商店（UWP）程序的窗口由 ApplicationFrameHost 托管：顶层窗口属于托管进程，
/// 程序自己的窗口是它的子窗口。按托管进程算的话，用户正在用的程序既不算前台、也不算有窗口，
/// 会被当成后台程序让路，所以换成子窗口所属的程序。
pub fn app_pid(hwnd: HWND) -> Option<Pid> {
    hosted_pid(hwnd).or_else(|| pid_of(hwnd))
}

/// 托管窗口（类名 ApplicationFrameWindow）里，属于别的进程的子窗口所属的进程；不是托管窗口、
/// 或程序已挂起（最小化后子窗口会被移走）时为 None。
fn hosted_pid(frame: HWND) -> Option<Pid> {
    unsafe extern "system" fn find(child: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam 是下面传入的 (托管进程, 结果) 指针，枚举期间有效。
        let (host, found) = unsafe { &mut *(lparam.0 as *mut (Pid, Option<Pid>)) };
        match pid_of(child) {
            Some(pid) if pid != *host => {
                *found = Some(pid);
                false.into()
            }
            _ => true.into(),
        }
    }
    if class_name(frame) != "ApplicationFrameWindow" {
        return None;
    }
    let mut state = (pid_of(frame)?, None);
    // SAFETY: 回调只在 EnumChildWindows 返回前被调用。
    let _ = unsafe {
        EnumChildWindows(
            Some(frame),
            Some(find),
            LPARAM(&mut state as *mut _ as isize),
        )
    };
    state.1
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
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            // 托管窗口：托管进程和程序自己都算有窗口。
            pids.extend(pid_of(hwnd));
            pids.extend(hosted_pid(hwnd));
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
    matches!(
        class_name(hwnd).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd"
    )
}

fn class_name(hwnd: HWND) -> String {
    let mut name = [0u16; 64];
    // SAFETY: 缓冲区长度与传入一致。
    let len = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
    String::from_utf16_lossy(&name[..len])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 开着的商店程序（如果有）：按程序自己的进程算，而不是托管进程。
    #[test]
    fn hosted_store_apps_resolve_to_their_own_process() {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam 是下面传入的 Vec 指针，枚举期间有效。
            let frames = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
            if class_name(hwnd) == "ApplicationFrameWindow" {
                frames.push(hwnd);
            }
            true.into()
        }
        let mut frames: Vec<HWND> = Vec::new();
        // SAFETY: 回调只在 EnumWindows 返回前被调用。
        let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut frames as *mut _ as isize)) };
        for frame in frames {
            if let Some(app) = hosted_pid(frame) {
                assert_ne!(Some(app), pid_of(frame));
                assert_eq!(app_pid(frame), Some(app));
            }
        }
        // 普通窗口按自己的进程算。
        let shell = unsafe { GetShellWindow() };
        assert_eq!(app_pid(shell), pid_of(shell));
    }
}
