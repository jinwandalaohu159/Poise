//! 托盘图标：添加、更新状态（图标与提示文字）、资源管理器重启后重新添加、移除。

mod icon;
pub mod menu;
pub mod promote;
pub mod theme;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};

use icon::Icons;

const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    callback_message: u32,
    icons: Icons,
    attention: bool,
    tooltip: String,
}

impl Tray {
    /// 添加托盘图标。图标上的鼠标操作以 `callback_message` 发给 `hwnd`。
    pub fn add(hwnd: HWND, callback_message: u32, tooltip: &str) -> Self {
        let tray = Self {
            hwnd,
            callback_message,
            icons: Icons::load(),
            attention: false,
            tooltip: tooltip.to_string(),
        };
        tray.register();
        tray
    }

    /// 资源管理器重启（任务栏重建）后需要重新添加。
    pub fn register(&self) {
        let mut data = self.data();
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: 结构体由 data() 完整初始化。
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// 更新图标（需要留意时带橙点）和提示文字；没变化时不调用系统。
    pub fn update(&mut self, attention: bool, tooltip: &str) {
        if attention == self.attention && tooltip == self.tooltip {
            return;
        }
        self.attention = attention;
        self.tooltip = tooltip.to_string();
        let data = self.data();
        // SAFETY: 同上。
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: self.callback_message,
            hIcon: if self.attention {
                self.icons.attention
            } else {
                self.icons.normal
            },
            ..Default::default()
        };
        let max = data.szTip.len() - 1;
        for (dst, src) in data
            .szTip
            .iter_mut()
            .zip(self.tooltip.encode_utf16().take(max))
        {
            *dst = src;
        }
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data();
        // SAFETY: 同上。
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}
