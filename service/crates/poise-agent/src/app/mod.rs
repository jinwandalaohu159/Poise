//! 托盘代理的主线程：一个不显示的窗口接收托盘、前台切换、状态更新、发现新版本的消息，把它们分派给各部分
//! （这里）；窗口本身、消息循环、前台切换钩子见 [`window`]。

mod window;

pub use window::run;

use poise_core::model::Pid;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::UI::Shell::{NIN_BALLOONUSERCLICK, NIN_SELECT};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, PBT_APMRESUMEAUTOMATIC, PostMessageW, RegisterWindowMessageW, SetTimer, WM_APP,
    WM_CLOSE, WM_CONTEXTMENU, WM_POWERBROADCAST, WM_TIMER, WM_WTSSESSION_CHANGE,
};
use windows::core::w;

use crate::foreground::LastForeground;
use crate::link::{Job, Link};
use crate::prefs::{self, UiPrefs};
use crate::session::desktop;
use crate::tray::menu::{Command, MenuState};
use crate::tray::promote::{self, Outcome};
use crate::tray::status::TrayStatus;
use crate::tray::{Tray, status, theme};
use crate::ui_app;
use crate::update::UpdateChecker;
use window::Reply;

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
pub(super) const WM_FOREGROUND: u32 = WM_APP + 3;
const WM_UPDATE: u32 = WM_APP + 4;
/// 界面保存了设置（见 [`prefs::watch`]）。
const WM_PREFS: u32 = WM_APP + 5;
/// 用键盘选中托盘图标（NIN_SELECT | NINF_KEY），windows crate 没有导出。
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
const TIMER_PROMOTE: usize = 1;
/// 托盘图标没加上（任务栏还没准备好）：隔一会儿再加。
const TIMER_TRAY: usize = 2;
const TRAY_RETRY_MS: u32 = 5_000;
/// 发现新版本时正在全屏玩游戏、演示：过一阵再提示。
const TIMER_ANNOUNCE: usize = 3;
const ANNOUNCE_RETRY_MS: u32 = 10 * 60_000;
/// 资源管理器为新图标建立设置需要一点时间：每 3 秒看一次，最多看 20 次。
const PROMOTE_EVERY_MS: u32 = 3_000;
const PROMOTE_MAX_TRIES: u32 = 20;
/// "临时全速"的时长（分钟）。
const BOOST_MINUTES: u32 = 60;

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

impl App {
    pub(super) fn create(hwnd: HWND) -> Self {
        let prefs = UiPrefs::load();
        theme::apply(prefs.theme);
        let raw = hwnd.0 as isize;
        let post = move |msg| {
            // SAFETY: 投递消息到本进程的窗口；窗口销毁后投递失败也无妨。
            let _ = unsafe { PostMessageW(Some(HWND(raw as _)), msg, WPARAM(0), LPARAM(0)) };
        };
        let link = Link::start(move || post(WM_STATUS));
        let updates = UpdateChecker::start(move || post(WM_UPDATE));
        prefs::watch(move || post(WM_PREFS));
        // SAFETY: 注册全局广播消息名；设置一次性检查定时器；订阅本会话的切换通知（锁屏、切换用户、远程连接）。
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        unsafe {
            SetTimer(Some(hwnd), TIMER_PROMOTE, PROMOTE_EVERY_MS, None);
            let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        }
        let tray = Tray::new(
            hwnd,
            WM_TRAY,
            &prefs.format("tray.tooltip", &prefs.get("status.disconnected")),
        );
        if !tray.register() {
            // SAFETY: 设置自己的定时器。
            unsafe { SetTimer(Some(hwnd), TIMER_TRAY, TRAY_RETRY_MS, None) };
        }
        let mut app = Self {
            hwnd,
            tray,
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

    pub(super) fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
        match msg {
            WM_TRAY => {
                // 第 4 版托盘消息：低 16 位是事件。一次左键单击会同时发来 WM_LBUTTONUP 和
                // NIN_SELECT，只响应 NIN_SELECT，否则面板刚打开就被第二次"开关"关上。
                match (lparam.0 & 0xFFFF) as u32 {
                    NIN_SELECT | NIN_KEYSELECT => ui_app::toggle_flyout(),
                    NIN_BALLOONUSERCLICK if self.notified.is_some() => ui_app::open_update(),
                    WM_CONTEXTMENU => return Reply::ShowMenu,
                    _ => {}
                }
            }
            WM_STATUS | WM_PREFS => self.refresh_icon(),
            WM_UPDATE => self.announce_update(),
            WM_FOREGROUND => {
                self.foreground.observe();
                self.link.send(Job::ForegroundChanged);
            }
            // 睡眠醒来、会话切换（解锁、切回这个用户、远程连上）：立即上报，不等下一次心跳。
            WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
                self.link.send(Job::Refresh);
                return Reply::Unhandled;
            }
            WM_WTSSESSION_CHANGE => self.link.send(Job::Refresh),
            WM_TIMER if wparam.0 == TIMER_PROMOTE => self.promote(),
            WM_TIMER if wparam.0 == TIMER_TRAY => {
                if self.tray.register() {
                    self.stop_timer(TIMER_TRAY);
                }
            }
            WM_TIMER if wparam.0 == TIMER_ANNOUNCE => {
                self.stop_timer(TIMER_ANNOUNCE);
                self.announce_update();
            }
            m if m == self.taskbar_created && m != 0 => {
                self.tray.register();
            }
            _ => return Reply::Unhandled,
        }
        Reply::Handled
    }

    /// 菜单要显示的内容（弹菜单之前取好，弹菜单期间不占着 App）。
    pub(super) fn prepare_menu(&self) -> MenuRequest {
        ui_app::reclaim_foreground();
        let prefs = UiPrefs::load();
        theme::apply(prefs.theme);
        MenuRequest {
            prefs,
            status: self.link.status(),
            target: self.foreground.target(),
            update: self.updates.available(),
        }
    }

    pub(super) fn run_command(&mut self, command: Option<Command>, target: Option<Pid>) {
        match command {
            Some(Command::SetMode(mode)) => self.link.send(Job::SetMode(mode)),
            Some(Command::Clean) => self.link.clean_now(),
            Some(Command::Boost) => {
                if let Some(pid) = target {
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
                // SAFETY: 投递到自己的窗口：处理完这条消息后再关闭，随后退出消息循环。
                let _ = unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            None => {}
        }
    }

    fn refresh_icon(&mut self) {
        let prefs = UiPrefs::load();
        let (attention, title) = status::describe(self.link.status().as_ref());
        self.tray.update(
            attention && prefs.tray_alert,
            &prefs.format("tray.tooltip", &prefs.get(title)),
        );
    }

    /// 发现新版本：弹一次通知（同一版本只提示一次），菜单里随后出现"更新"。正在全屏玩游戏、演示、
    /// 专注时不弹，过一阵再看。
    fn announce_update(&mut self) {
        let Some(version) = self.updates.available() else {
            return;
        };
        if self.notified.as_deref() == Some(version.as_str()) {
            return;
        }
        if !desktop::notifications_welcome() {
            // SAFETY: 设置自己的定时器（已有同名定时器时重设）。
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANNOUNCE, ANNOUNCE_RETRY_MS, None) };
            return;
        }
        let prefs = UiPrefs::load();
        let shown = self.tray.notify(
            &prefs.get("tray.update.title"),
            &prefs.format("tray.update.body", &format!("Poise {version}")),
        );
        // 没弹出来（例如任务栏还没准备好）时，过一阵再试。
        if shown {
            self.notified = Some(version);
        } else {
            // SAFETY: 同上。
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANNOUNCE, ANNOUNCE_RETRY_MS, None) };
        }
    }

    fn stop_timer(&self, id: usize) {
        // SAFETY: 停止自己的定时器。
        let _ = unsafe { KillTimer(Some(self.hwnd), id) };
    }

    fn promote(&mut self) {
        self.promote_tries += 1;
        if matches!(promote::once(), Outcome::Done) || self.promote_tries >= PROMOTE_MAX_TRIES {
            self.stop_timer(TIMER_PROMOTE);
        }
    }
}

/// 弹菜单前取好的内容。
pub(super) struct MenuRequest {
    prefs: UiPrefs,
    status: Option<TrayStatus>,
    pub(super) target: Option<(Pid, String)>,
    update: Option<String>,
}

impl MenuRequest {
    pub(super) fn state(&self) -> MenuState<'_> {
        let status = self.status.as_ref();
        MenuState {
            prefs: &self.prefs,
            connected: status.is_some(),
            mode: status.map(|s| s.mode),
            vendor_mode: status.and_then(|s| s.hardware.vendor_mode.clone()),
            boost_target: self.target.as_ref().map(|(_, name)| name.clone()),
            update: self.update.clone(),
        }
    }
}
