//! 托盘右键菜单是系统原生菜单，默认总是浅色。这里让本进程的原生菜单跟随 Poise 的深浅色设置。
//! 只影响 Poise 自己的菜单；系统不支持时保持浅色，不影响功能。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

use crate::prefs::Theme;

/// uxtheme 按序号导出（Windows 10 1903 起），没有公开的名字。
const SET_PREFERRED_APP_MODE: usize = 135;
const FLUSH_MENU_THEMES: usize = 136;

const ALLOW_DARK: i32 = 1;
const FORCE_DARK: i32 = 2;
const FORCE_LIGHT: i32 = 3;

/// uxtheme 的两个导出函数（加载一次，进程结束前一直有效）。
struct Exports {
    set_mode: extern "system" fn(i32) -> i32,
    flush: extern "system" fn(),
}

fn exports() -> Option<&'static Exports> {
    static EXPORTS: OnceLock<Option<Exports>> = OnceLock::new();
    EXPORTS
        .get_or_init(|| {
            // SAFETY: 按序号取得 uxtheme 的导出函数；签名与系统一致（一个 int 参数 / 无参数）。
            // 库不释放：函数在进程结束前都要用。
            unsafe {
                let uxtheme = LoadLibraryW(w!("uxtheme.dll")).ok()?;
                let set = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE as *const u8))?;
                let flush = GetProcAddress(uxtheme, PCSTR(FLUSH_MENU_THEMES as *const u8))?;
                Some(Exports {
                    set_mode: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        extern "system" fn(i32) -> i32,
                    >(set),
                    flush: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        extern "system" fn(),
                    >(flush),
                })
            }
        })
        .as_ref()
}

/// 每次弹菜单前调用；和上次一样时不重复设置。
pub fn apply(theme: Theme) {
    static APPLIED: AtomicI32 = AtomicI32::new(0);
    let mode = match theme {
        Theme::System => ALLOW_DARK,
        Theme::Dark => FORCE_DARK,
        Theme::Light => FORCE_LIGHT,
    };
    if APPLIED.swap(mode, Ordering::Relaxed) == mode {
        return;
    }
    if let Some(e) = exports() {
        (e.set_mode)(mode);
        (e.flush)();
    }
}
