//! 托盘右键菜单：模式（自动、安静、均衡、全力，机型支持时加极限模式）、一键清理、让某程序全速、打开、退出。

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

/// 所有机型都有的模式；机型极限模式另外追加。
const MODES: [Mode; 4] = [Mode::Auto, Mode::Quiet, Mode::Balanced, Mode::Full];
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
    /// 机型极限模式的标识（例如 "dell_g_mode"）；不支持时为 None。
    pub vendor_mode: Option<String>,
    /// 可以"临时全速"的程序名。
    pub boost_target: Option<String>,
}

/// 在鼠标位置弹出菜单，返回用户选择的命令。
pub fn show(hwnd: HWND, state: &MenuState) -> Option<Command> {
    // SAFETY: 菜单在本函数内创建并销毁；SetForegroundWindow 是弹出菜单的官方要求（否则点别处不会消失）。
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let modes = modes(state);
        build(menu, state, &modes);
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
        command_of(id, &modes)
    }
}

fn modes(s: &MenuState) -> Vec<Mode> {
    let mut modes = MODES.to_vec();
    if s.vendor_mode.is_some() {
        modes.push(Mode::Vendor);
    }
    modes
}

/// 菜单项的文字键。极限模式按机型显示名称（"G 模式"）。
fn label_key(mode: Mode, vendor: Option<&str>) -> String {
    match mode {
        Mode::Auto => "mode.auto".into(),
        Mode::Quiet => "mode.quiet".into(),
        Mode::Balanced => "mode.balanced".into(),
        Mode::Full => "mode.full".into(),
        Mode::Vendor => format!("mode.vendor.{}", vendor.unwrap_or_default()),
    }
}

fn command_of(id: u32, modes: &[Mode]) -> Option<Command> {
    match id {
        1..=9 => modes.get(id as usize - 1).map(|&m| Command::SetMode(m)),
        ID_CLEAN => Some(Command::Clean),
        ID_BOOST => Some(Command::Boost),
        ID_OPEN => Some(Command::Open),
        ID_EXIT => Some(Command::Exit),
        _ => None,
    }
}

unsafe fn build(menu: HMENU, s: &MenuState, modes: &[Mode]) {
    let p = s.prefs;
    let enabled = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };
    for (i, &mode) in modes.iter().enumerate() {
        let mut flags = enabled(s.connected);
        if s.mode == Some(mode) {
            flags |= MF_CHECKED;
        }
        let label = p.get(&label_key(mode, s.vendor_mode.as_deref()));
        unsafe { append(menu, flags, i as u32 + 1, &label) };
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
