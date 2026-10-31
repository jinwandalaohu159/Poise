//! 服务收到的电源事件：熄屏、亮屏、挂起、醒来，换成引擎的 [`PowerChange`]（见 `engine::standby`）；
//! 屏幕亮灭要另外订阅（[`subscribe_display`]）。

use std::os::windows::io::AsRawHandle;

use poise_platform::win::power_notify::DisplayNotify;
use windows_service::service::{DisplayState, PowerBroadcastSetting, PowerEventParam};
use windows_service::service_control_handler::ServiceStatusHandle;

use crate::engine::PowerChange;

/// 服务关心的电源事件；其余为 None。
pub fn power_change(param: PowerEventParam) -> Option<PowerChange> {
    match param {
        PowerEventParam::Suspend => Some(PowerChange::Suspending),
        PowerEventParam::ResumeAutomatic => Some(PowerChange::Resumed),
        PowerEventParam::PowerSettingChange(PowerBroadcastSetting::ConsoleDisplayState(state)) => {
            Some(if state == DisplayState::Off {
                PowerChange::DisplayOff
            } else {
                PowerChange::DisplayOn
            })
        }
        _ => None,
    }
}

/// 订阅屏幕亮灭（订阅时立即收到一次当前状态），留着返回值直到服务停止。订不上只是待机时按旧办法处理
/// （托盘代理没上报就暂停）。
pub fn subscribe_display(handle: &ServiceStatusHandle) -> Option<DisplayNotify> {
    DisplayNotify::for_service(handle.as_raw_handle() as isize)
        .inspect_err(|e| log::warn!("订阅屏幕亮灭通知失败：{e}"))
        .ok()
}
