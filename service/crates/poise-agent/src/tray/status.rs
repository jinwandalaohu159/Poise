//! 托盘图标上的状态：要不要显示提醒圆点，以及提示文字里的状态词。
//!
//! 托盘只读状态里用得到的几项（[`TrayStatus`]），不解析整份状态（走势、硬件读数、清理记录……）：
//! 每几秒取一次，常驻的托盘代理不必每次都建一整棵结构。

use poise_core::diagnose::{HealthCode, Level};
use poise_core::prefs::Mode;
use serde::Deserialize;

/// 托盘要的状态：健康状况、模式、机型极限模式（菜单里显示它的名字）。
#[derive(Debug, Clone, Deserialize)]
pub struct TrayStatus {
    pub health: TrayHealth,
    pub mode: Mode,
    #[serde(default)]
    pub hardware: TrayHardware,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TrayHealth {
    pub code: HealthCode,
    pub level: Level,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrayHardware {
    #[serde(default)]
    pub vendor_mode: Option<String>,
}

/// 状态 → （是否需要留意，状态词的文字键）。没连上服务时不提醒，但写明"未连接"。
pub fn describe(status: Option<&TrayStatus>) -> (bool, &'static str) {
    let Some(s) = status else {
        return (false, "status.disconnected");
    };
    let key = match s.health.code {
        HealthCode::Smooth => "status.smooth",
        HealthCode::InGame => "status.in_game",
        HealthCode::Tired => "status.tired",
        HealthCode::Hot => "status.hot",
        HealthCode::RestartRecommended => "status.restart",
        HealthCode::StorageLow => "status.storage",
    };
    (matches!(s.health.level, Level::Notice | Level::Bad), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_is_quiet_but_labelled() {
        assert_eq!(describe(None), (false, "status.disconnected"));
    }

    #[test]
    fn only_the_parts_the_tray_shows_are_read_from_a_full_status() {
        let full = r#"{"health":{"code":"hot","level":"bad","game":null,"cleanable":0,"freeable_bytes":0},
            "mode":"full","effective_mode":"full","metrics":{"cpu_pct":1.0},
            "hardware":{"profile":"x","vendor_mode":"dell_g_mode","fans":[]}}"#;
        let s: TrayStatus = serde_json::from_str(full).unwrap();
        assert_eq!(describe(Some(&s)), (true, "status.hot"));
        assert_eq!(s.mode, Mode::Full);
        assert_eq!(s.hardware.vendor_mode.as_deref(), Some("dell_g_mode"));
    }
}
