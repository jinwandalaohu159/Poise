//! 托盘右键菜单：三档模式、一键清理、让某程序全速、打开、退出。

use poise_core::prefs::Mode;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, MENU_ITEM_FLAGS, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};

use crate::prefs::UiPrefs;
use poise_platform::win::wide::WideString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetMode(Mode),
    Clean,
    Boost,
    Open,
    Exit,
}

const MODES: [Mode; 3] = [Mode::Quiet, Mode::Balanced, Mode::Full];
const ID_CLEAN: u32 = 10;
const ID_BOOST: u32 = 11;
const ID_OPEN: u32 = 20;
const ID_EXIT: u32 = 21;

/// 菜单此刻应显示的内容。
pub struct MenuState<'a> {
    pub prefs: &'a UiPrefs,
    /// 服务连着时才能切换模式和清理。
    pub connected: bool,
    pub mode: Option<Mode>,
    /// 可以"临时全速"的程序名。
    pub boost_target: Option<String>,
}

/// 在鼠标位置弹出菜单，返回用户选择的命令。
pub fn show(hwnd: HWND, state: &MenuState) -> Option<Command> {
    // SAFETY: 菜单在本函数内创建并销毁；SetForegroundWindow 是弹出菜单的官方要求（否则点别处不会消失）。
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        build(menu, state);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            pt.x,
            pt.y,
            hwnd,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        command_of(id)
    }
}

fn command_of(id: u32) -> Option<Command> {
    match id {
        1..=3 => Some(Command::SetMode(MODES[id as usize - 1])),
        ID_CLEAN => Some(Command::Clean),
        ID_BOOST => Some(Command::Boost),
        ID_OPEN => Some(Command::Open),
        ID_EXIT => Some(Command::Exit),
        _ => None,
    }
}

unsafe fn build(menu: HMENU, s: &MenuState) {
    let p = s.prefs;
    let enabled = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };
    for (i, mode) in MODES.iter().enumerate() {
        let key = match mode {
            Mode::Quiet => "mode.quiet",
            Mode::Balanced => "mode.balanced",
            Mode::Full => "mode.full",
        };
        let mut flags = enabled(s.connected);
        if s.mode == Some(*mode) {
            flags |= MF_CHECKED;
        }
        unsafe { append(menu, flags, i as u32 + 1, &p.get(key)) };
    }
    let boost = match &s.boost_target {
        Some(name) => p.format("tray.boost", name),
        None => p.get("tray.boost_none"),
    };
    unsafe {
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, enabled(s.connected), ID_CLEAN, &p.get("home.clean"));
        append(
            menu,
            enabled(s.connected && s.boost_target.is_some()),
            ID_BOOST,
            &boost,
        );
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, ID_OPEN, &p.get("tray.open"));
        append(menu, MF_STRING, ID_EXIT, &p.get("tray.exit"));
    }
}

unsafe fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: u32, text: &str) {
    let text = WideString::new(text);
    // SAFETY: 文本在调用期间有效，菜单会复制它。
    let _ = unsafe { AppendMenuW(menu, flags, id as usize, text.as_pcwstr()) };
}
