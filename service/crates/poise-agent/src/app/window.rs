//! 托盘代理的窗口：注册窗口类、建一个不显示的顶层窗口（只有它能收到"任务栏已重建"的广播）、
//! 挂上前台切换钩子、跑消息循环；窗口过程把消息交给 [`App`] 处理。
//!
//! 右键菜单弹出期间，菜单的消息循环会把别的消息（前台切换、状态更新、定时器）再送进窗口过程。
//! 所以弹菜单时不占着 [`App`]：先取好菜单要显示的内容，放开，弹菜单，回来再执行选中的命令
//! （见 [`window_proc`]）。退出也是投递关闭消息，不在处理消息的中途销毁窗口。

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, MSG, OBJID_WINDOW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, TranslateMessage,
    WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CREATE, WM_DESTROY,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::w;

use super::{App, WM_FOREGROUND};
use crate::tray::menu;

/// 前台切换钩子的回调没有上下文参数，用它找到窗口。
static WINDOW: AtomicIsize = AtomicIsize::new(0);

pub fn run() {
    // SAFETY: 标准的窗口类注册与创建；窗口过程通过 GWLP_USERDATA 取得 App。
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("PoiseAgent"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // 普通的隐藏顶层窗口（不显示）：只有它能收到"任务栏已重建"的广播。
        let Ok(_window) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("PoiseAgent"),
            w!("Poise"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            log::error!("创建窗口失败");
            return;
        };
        // 自己的窗口（托盘菜单）成为前台不算前台切换。
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let hooks = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND]
            .map(|e| SetWinEventHook(e, e, None, Some(on_win_event), 0, 0, flags));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
    }
}

/// 处理一条消息之后要做什么。
pub(super) enum Reply {
    Handled,
    Unhandled,
    /// 弹出右键菜单（不占着 App，见模块说明）。
    ShowMenu,
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let raw = WINDOW.load(Ordering::Relaxed);
    // 还有一条没处理的就不再投递：事件扎堆（切换窗口、最小化动画）时托盘线程只处理一次。
    if raw != 0 && object == OBJID_WINDOW.0 && !FOREGROUND_PENDING.swap(true, Ordering::AcqRel) {
        // SAFETY: 投递到本进程的窗口。
        let posted =
            unsafe { PostMessageW(Some(HWND(raw as _)), WM_FOREGROUND, WPARAM(0), LPARAM(0)) };
        if posted.is_err() {
            FOREGROUND_PENDING.store(false, Ordering::Release);
        }
    }
}

/// 有一条前台切换的消息投递了还没处理。
static FOREGROUND_PENDING: AtomicBool = AtomicBool::new(false);

/// 开始处理前台切换的消息：之后的事件再投递新的一条。
pub(super) fn foreground_taken() {
    FOREGROUND_PENDING.store(false, Ordering::Release);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA 里是 WM_CREATE 时放入的 RefCell<App>，窗口销毁时取回释放。
    // 销毁只经由投递的关闭消息发生，那时没有任何借用。
    unsafe {
        match msg {
            WM_CREATE => {
                WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
                let app = Box::new(RefCell::new(App::create(hwnd)));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app) as isize);
                return LRESULT(0);
            }
            WM_DESTROY => {
                let _ = WTSUnRegisterSessionNotification(hwnd);
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut RefCell<App>;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                WINDOW.store(0, Ordering::Relaxed);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        let Some(cell) = ptr.as_ref() else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        // 借不到说明正在处理别的消息时又被送进来（不应发生）：交给系统默认处理。
        let reply = match cell.try_borrow_mut() {
            Ok(mut app) => app.on_message(msg, wparam, lparam),
            Err(_) => Reply::Unhandled,
        };
        match reply {
            Reply::Handled => LRESULT(0),
            Reply::Unhandled => DefWindowProcW(hwnd, msg, wparam, lparam),
            Reply::ShowMenu => {
                let Ok(request) = cell.try_borrow().map(|app| app.prepare_menu()) else {
                    return LRESULT(0);
                };
                let command = menu::show(hwnd, &request.state());
                if let Ok(mut app) = cell.try_borrow_mut() {
                    app.run_command(command, request.target.map(|(pid, _)| pid));
                }
                LRESULT(0)
            }
        }
    }
}
