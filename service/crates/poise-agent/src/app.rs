//! 托盘代理的主线程：一个不显示的窗口接收托盘、前台切换、状态更新、发现新版本的消息，把它们分派给各部分。

use std::sync::atomic::{AtomicIsize, Ordering};

use poise_core::diagnose::{HealthCode, Level};
use poise_protocol::Status;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::{NIN_BALLOONUSERCLICK, NIN_SELECT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, KillTimer, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetTimer,
    SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT, WM_APP,
    WM_CONTEXTMENU, WM_CREATE, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::w;

use crate::foreground::LastForeground;
use crate::link::{Job, Link};
use crate::prefs::UiPrefs;
use crate::tray::menu::{self, Command, MenuState};
use crate::tray::promote::{self, Outcome};
use crate::tray::{Tray, theme};
use crate::ui_app;
use crate::update::UpdateChecker;

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
const WM_FOREGROUND: u32 = WM_APP + 3;
const WM_UPDATE: u32 = WM_APP + 4;
/// 用键盘选中托盘图标（NIN_SELECT | NINF_KEY），windows crate 没有导出。
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
const TIMER_PROMOTE: usize = 1;
/// 资源管理器为新图标建立设置需要一点时间：每 3 秒看一次，最多看 20 次。
const PROMOTE_EVERY_MS: u32 = 3_000;
const PROMOTE_MAX_TRIES: u32 = 20;
/// "临时全速"的时长（分钟）。
const BOOST_MINUTES: u32 = 60;

/// 前台切换钩子的回调没有上下文参数，用它找到窗口。
static WINDOW: AtomicIsize = AtomicIsize::new(0);

struct App {
    hwnd: HWND,
    tray: Tray,
    link: Link,
    foreground: LastForeground,
    taskbar_created: u32,
    promote_tries: u32,
    updates: UpdateChecker,
    /// 已经弹过通知的新版本（每个版本只提示一次）。
    notified: Option<String>,
}

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
        let hooks = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND]
            .map(|e| SetWinEventHook(e, e, None, Some(on_win_event), 0, 0, WINEVENT_OUTOFCONTEXT));
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

impl App {
    fn create(hwnd: HWND) -> Self {
        WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
        let prefs = UiPrefs::load();
        theme::apply(prefs.theme);
        let raw = hwnd.0 as isize;
        let post = move |msg| {
            // SAFETY: 投递消息到本进程的窗口；窗口销毁后投递失败也无妨。
            let _ = unsafe { PostMessageW(Some(HWND(raw as _)), msg, WPARAM(0), LPARAM(0)) };
        };
        let link = Link::start(move || post(WM_STATUS));
        let updates = UpdateChecker::start(move || post(WM_UPDATE));
        // SAFETY: 注册全局广播消息名；设置一次性检查定时器。
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        unsafe { SetTimer(Some(hwnd), TIMER_PROMOTE, PROMOTE_EVERY_MS, None) };
        let mut app = Self {
            hwnd,
            tray: Tray::add(
                hwnd,
                WM_TRAY,
                &prefs.format("tray.tooltip", &prefs.get("status.disconnected")),
            ),
            link,
            foreground: LastForeground::default(),
            taskbar_created,
            promote_tries: 0,
            updates,
            notified: None,
        };
        app.foreground.observe();
        app
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_TRAY => {
                // 第 4 版托盘消息：低 16 位是事件。一次左键单击会同时发来 WM_LBUTTONUP 和
                // NIN_SELECT，只响应 NIN_SELECT，否则面板刚打开就被第二次"开关"关上。
                match (lparam.0 & 0xFFFF) as u32 {
                    NIN_SELECT | NIN_KEYSELECT => ui_app::toggle_flyout(),
                    NIN_BALLOONUSERCLICK if self.notified.is_some() => ui_app::open_update(),
                    WM_CONTEXTMENU => self.show_menu(),
                    _ => {}
                }
            }
            WM_STATUS => self.refresh_icon(),
            WM_UPDATE => self.announce_update(),
            WM_FOREGROUND => {
                self.foreground.observe();
                self.link.send(Job::ForegroundChanged);
            }
            WM_TIMER if wparam.0 == TIMER_PROMOTE => self.promote(),
            m if m == self.taskbar_created && m != 0 => self.tray.register(),
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn show_menu(&mut self) {
        ui_app::reclaim_foreground();
        let prefs = UiPrefs::load();
        theme::apply(prefs.theme);
        let status = self.link.status();
        let target = self.foreground.target();
        let state = MenuState {
            prefs: &prefs,
            connected: status.is_some(),
            mode: status.as_ref().map(|s| s.mode),
            vendor_mode: status.as_ref().and_then(|s| s.hardware.vendor_mode.clone()),
            boost_target: target.as_ref().map(|(_, name)| name.clone()),
            update: self.updates.available(),
        };
        match menu::show(self.hwnd, &state) {
            Some(Command::SetMode(mode)) => self.link.send(Job::SetMode(mode)),
            Some(Command::Clean) => self.link.send(Job::CleanNow),
            Some(Command::Boost) => {
                if let Some((pid, _)) = target {
                    self.link.send(Job::Boost {
                        pid,
                        minutes: BOOST_MINUTES,
                    });
                }
            }
            Some(Command::Update) => ui_app::open_update(),
            Some(Command::Open) => ui_app::open_window(),
            Some(Command::Exit) => {
                ui_app::exit();
                // SAFETY: 销毁自己的窗口，随后退出消息循环。
                let _ = unsafe { DestroyWindow(self.hwnd) };
            }
            None => {}
        }
    }

    fn refresh_icon(&mut self) {
        let prefs = UiPrefs::load();
        let (attention, title) = describe(self.link.status().as_ref());
        self.tray.update(
            attention && prefs.tray_alert,
            &prefs.format("tray.tooltip", &prefs.get(title)),
        );
    }

    /// 发现新版本：弹一次通知（同一版本只提示一次），菜单里随后出现"更新"。
    fn announce_update(&mut self) {
        let Some(version) = self.updates.available() else {
            return;
        };
        if self.notified.as_deref() == Some(version.as_str()) {
            return;
        }
        let prefs = UiPrefs::load();
        let shown = self.tray.notify(
            &prefs.get("tray.update.title"),
            &prefs.format("tray.update.body", &format!("Poise {version}")),
        );
        // 没弹出来（例如任务栏还没准备好）时，下次发现新版本再试。
        if shown {
            self.notified = Some(version);
        }
    }

    fn promote(&mut self) {
        self.promote_tries += 1;
        if matches!(promote::once(), Outcome::Done) || self.promote_tries >= PROMOTE_MAX_TRIES {
            // SAFETY: 停止自己的定时器。
            let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_PROMOTE) };
        }
    }
}

/// 状态 → （是否需要留意，状态词的文字键）。
fn describe(status: Option<&Status>) -> (bool, &'static str) {
    let Some(s) = status else {
        return (false, "status.disconnected");
    };
    let key = match s.health.code {
        HealthCode::Smooth => "status.smooth",
        HealthCode::InGame => "status.in_game",
        HealthCode::Tired => "status.tired",
        HealthCode::Hot => "status.hot",
        HealthCode::RestartRecommended => "status.restart",
        HealthCode::StorageLow => "status.storage",
    };
    (matches!(s.health.level, Level::Notice | Level::Bad), key)
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let raw = WINDOW.load(Ordering::Relaxed);
    if raw != 0 {
        // SAFETY: 投递到本进程的窗口。
        let _ = unsafe { PostMessageW(Some(HWND(raw as _)), WM_FOREGROUND, WPARAM(0), LPARAM(0)) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA 里是 WM_CREATE 时放入的 App，窗口销毁时取回释放。
    unsafe {
        match msg {
            WM_CREATE => {
                let app = Box::new(App::create(hwnd));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app) as isize);
                return LRESULT(0);
            }
            WM_DESTROY => {
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                WINDOW.store(0, Ordering::Relaxed);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if let Some(app) = ptr.as_mut()
            && let Some(result) = app.on_message(msg, wparam, lparam)
        {
            return result;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_is_quiet_but_labelled() {
        assert_eq!(describe(None), (false, "status.disconnected"));
    }
}
