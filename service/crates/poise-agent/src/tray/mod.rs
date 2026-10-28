//! 托盘图标：添加、更新状态（图标与提示文字）、弹出通知、资源管理器重启后重新添加、移除。

mod icon;
pub mod menu;
pub mod promote;
pub mod theme;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
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

    /// 弹出一条系统通知（点击时托盘消息是 NIN_BALLOONUSERCLICK）。
    /// 弹出系统通知，返回是否成功。
    pub fn notify(&self, title: &str, body: &str) -> bool {
        let mut data = self.data();
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_text(&mut data.szInfoTitle, title);
        copy_text(&mut data.szInfo, body);
        // SAFETY: 同上。
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
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
        copy_text(&mut data.szTip, &self.tooltip);
        data
    }
}

/// 把文字放进定长缓冲区，超长时截断，保留结尾的 NUL。
fn copy_text(dst: &mut [u16], text: &str) {
    let max = dst.len() - 1;
    for (d, s) in dst.iter_mut().zip(text.encode_utf16().take(max)) {
        *d = s;
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data();
        // SAFETY: 同上。
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}
