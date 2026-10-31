//! 屏幕亮灭的通知（`GUID_CONSOLE_DISPLAY_STATE`）。现代待机（S0 低功耗待机，合盖、按电源键）不像传统
//! 睡眠那样把整台电脑停下：先熄屏，桌面程序随后被系统冻结，服务照常运行。熄屏是所有电脑都有、最早到来的
//! 信号，服务和托盘代理都按它判断"没人在用桌面"；传统睡眠另有挂起、唤醒的通知。

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::System::Power::{
    HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
    UnregisterPowerSettingNotification,
};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_SERVICE_HANDLE, DEVICE_NOTIFY_WINDOW_HANDLE, REGISTER_NOTIFICATION_FLAGS,
};

/// 订阅着屏幕亮灭的通知；丢弃时取消订阅。
pub struct DisplayNotify(HPOWERNOTIFY);

// SAFETY: 订阅句柄只用来取消订阅，可以在任何线程上做。
unsafe impl Send for DisplayNotify {}
unsafe impl Sync for DisplayNotify {}

impl DisplayNotify {
    /// 服务订阅：通知以服务控制（电源事件）送达。`status_handle` 是服务管理器给的状态句柄。
    pub fn for_service(status_handle: isize) -> windows::core::Result<Self> {
        Self::register(HANDLE(status_handle as _), DEVICE_NOTIFY_SERVICE_HANDLE)
    }

    /// 窗口订阅：通知以 `WM_POWERBROADCAST`（`PBT_POWERSETTINGCHANGE`）送达，用 [`display_on`] 解读。
    pub fn for_window(hwnd: HWND) -> windows::core::Result<Self> {
        Self::register(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
    }

    fn register(
        recipient: HANDLE,
        flags: REGISTER_NOTIFICATION_FLAGS,
    ) -> windows::core::Result<Self> {
        // SAFETY: 接收者是调用方持有的有效句柄；GUID 是常量。
        unsafe { RegisterPowerSettingNotification(recipient, &GUID_CONSOLE_DISPLAY_STATE, flags) }
            .map(Self)
    }
}

impl Drop for DisplayNotify {
    fn drop(&mut self) {
        // SAFETY: 订阅时得到的句柄，只取消一次。
        let _ = unsafe { UnregisterPowerSettingNotification(self.0) };
    }
}

/// 解读窗口收到的 `PBT_POWERSETTINGCHANGE`：屏幕亮着（变暗也算亮着）为 Some(true)，熄了为 Some(false)；
/// 不是屏幕亮灭的通知为 None。
pub fn display_on(lparam: LPARAM) -> Option<bool> {
    if lparam.0 == 0 {
        return None;
    }
    // SAFETY: 系统随 PBT_POWERSETTINGCHANGE 给的 POWERBROADCAST_SETTING，消息处理期间有效。
    let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
    if setting.PowerSetting != GUID_CONSOLE_DISPLAY_STATE || setting.DataLength < 1 {
        return None;
    }
    // 0 熄屏、1 亮着、2 变暗。
    Some(setting.Data[0] != 0)
}
