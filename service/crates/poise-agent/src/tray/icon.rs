//! 托盘图标：嵌入的 .ico 按当前缩放取合适的尺寸。

use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR, LookupIconIdFromDirectoryEx,
    SM_CXSMICON,
};

const NORMAL: &[u8] = include_bytes!("../../../../../app/Poise.App/Assets/Poise.ico");
const ATTENTION: &[u8] = include_bytes!("../../../../../app/Poise.App/Assets/PoiseAttention.ico");

/// 一对图标：正常，需要留意（带橙点）。
pub struct Icons {
    pub normal: HICON,
    pub attention: HICON,
}

impl Icons {
    pub fn load() -> Self {
        // SAFETY: 无参数。
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
        Self {
            normal: from_ico(NORMAL, size),
            attention: from_ico(ATTENTION, size),
        }
    }
}

impl Drop for Icons {
    fn drop(&mut self) {
        // SAFETY: 图标由本结构体创建并独占。
        unsafe {
            let _ = DestroyIcon(self.normal);
            let _ = DestroyIcon(self.attention);
        }
    }
}

/// 从 .ico 文件内容里挑出最接近 `size` 的一张，创建图标。
fn from_ico(ico: &[u8], size: i32) -> HICON {
    // SAFETY: 缓冲区是完整的 .ico 文件内容，偏移量由系统从目录中算出。
    unsafe {
        let offset = LookupIconIdFromDirectoryEx(ico.as_ptr(), true, size, size, LR_DEFAULTCOLOR);
        let image = ico.get(offset.max(0) as usize..).unwrap_or(ico);
        CreateIconFromResourceEx(image, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR)
            .unwrap_or_default()
    }
}
