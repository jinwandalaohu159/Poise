//! 托盘右键菜单是系统原生菜单，默认总是浅色。这里让本进程的原生菜单跟随 Poise 的深浅色设置。
//! 只影响 Poise 自己的菜单；系统不支持时保持浅色，不影响功能。

use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

use crate::prefs::Theme;

/// uxtheme 按序号导出（Windows 10 1903 起），没有公开的名字。
const SET_PREFERRED_APP_MODE: usize = 135;
const FLUSH_MENU_THEMES: usize = 136;

const ALLOW_DARK: i32 = 1;
const FORCE_DARK: i32 = 2;
const FORCE_LIGHT: i32 = 3;

pub fn apply(theme: Theme) {
    let mode = match theme {
        Theme::System => ALLOW_DARK,
        Theme::Dark => FORCE_DARK,
        Theme::Light => FORCE_LIGHT,
    };
    // SAFETY: 按序号取得 uxtheme 的导出函数；签名与系统一致（一个 int 参数 / 无参数）。
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        let set = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE as *const u8));
        let flush = GetProcAddress(uxtheme, PCSTR(FLUSH_MENU_THEMES as *const u8));
        if let (Some(set), Some(flush)) = (set, flush) {
            let set: extern "system" fn(i32) -> i32 = std::mem::transmute(set);
            let flush: extern "system" fn() = std::mem::transmute(flush);
            set(mode);
            flush();
        }
    }
}
