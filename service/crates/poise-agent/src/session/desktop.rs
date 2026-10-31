//! 桌面上的状态：前台窗口、全屏、可见窗口、桌面外壳、输入空闲时间。只读。

use std::collections::HashSet;

use poise_core::model::Pid;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::RemoteDesktop::{
    WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SESSION, WTSActive, WTSConnectState, WTSFreeMemory,
    WTSQuerySessionInformationW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_ACCEPTS_NOTIFICATIONS, QUNS_NOT_PRESENT, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow,
    GetShellWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindow,
    IsWindowVisible, IsZoomed, WS_CAPTION, WS_EX_TOOLWINDOW,
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

/// 前台窗口铺满它所在的显示器：独占全屏，或无边框窗口铺满整个显示器（多数游戏的"全屏"）。
/// 只看前台窗口自己：系统的"有程序在全屏"是全局的（副屏在全屏放视频时，主屏上的程序也会被算进去）。
/// 有标题栏、又是最大化的窗口（任务栏自动隐藏时会铺满屏幕）不算；有些无边框游戏保留了标题栏样式
/// （自己去掉边框），没有最大化，照样算。
pub fn is_fullscreen(foreground: HWND) -> bool {
    if foreground.is_invalid() || is_desktop(foreground) {
        return false;
    }
    // SAFETY: 读取窗口状态。
    if has_caption(foreground) && unsafe { IsZoomed(foreground) }.as_bool() {
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

/// 切换过程中短暂成为前台的窗口：Alt+Tab 和任务视图、切换前台时的过渡窗口、任务栏与托盘溢出区，
/// 以及工具窗口（浮动面板、通知）。它们不算"用户切到了别的程序"，前台沿用之前的。
pub fn is_transient(hwnd: HWND) -> bool {
    // SAFETY: 读取窗口样式。
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return true;
    }
    matches!(
        class_name(hwnd).as_str(),
        "ForegroundStaging"
            | "XamlExplorerHostIslandWindow"
            | "MultitaskingViewFrame"
            | "TaskSwitcherWnd"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TopLevelWindowForOverflowXamlIsland"
    )
}

/// 本会话是不是正在使用的那个（控制台上没切走、远程连接没断开）。查不到时按是。
pub fn session_active() -> bool {
    // 本机最常见的情况：本会话就是控制台的活动会话（只读共享内存，不跨进程）。远程会话再去问会话服务。
    // SAFETY: 无参数。
    if unsafe { windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() }
        == super::current_session()
    {
        return true;
    }
    let mut buf = windows::core::PWSTR::null();
    let mut len = 0u32;
    // SAFETY: 输出由系统分配，用完用 WTSFreeMemory 释放；连接状态是一个 4 字节的枚举值。
    unsafe {
        if WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSConnectState,
            &mut buf,
            &mut len,
        )
        .is_err()
            || buf.is_null()
        {
            return true;
        }
        let active = len as usize >= size_of::<WTS_CONNECTSTATE_CLASS>()
            && *(buf.0 as *const WTS_CONNECTSTATE_CLASS) == WTSActive;
        WTSFreeMemory(buf.0.cast());
        active
    }
}

/// 窗口还在（程序没关掉它）。
pub fn window_exists(hwnd: HWND) -> bool {
    // SAFETY: 只读取窗口是否存在。
    !hwnd.is_invalid() && unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

/// 现在弹通知不打扰人：没在全屏玩游戏、演示、专注，也没离开。
pub fn notifications_welcome() -> bool {
    // SAFETY: 输出到本地变量。
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_ACCEPTS_NOTIFICATIONS)
}

pub fn desktop_away() -> bool {
    // SAFETY: 输出到本地变量。
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_NOT_PRESENT)
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
