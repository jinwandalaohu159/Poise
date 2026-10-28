//! 界面的语言、外观与托盘提醒设置（界面写在 %LOCALAPPDATA%\Poise\ui.json），以及对应的文字表。
//! 文字表与界面共用同一份文件，编译时嵌入。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use windows::Win32::Globalization::GetUserDefaultUILanguage;

const ZH: &str = include_str!("../../../../app/Poise.Core/Localization/Strings/zh-CN.json");
const EN: &str = include_str!("../../../../app/Poise.Core/Localization/Strings/en-US.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct UiFile {
    language: Option<String>,
    theme: u8,
    tray_alert: bool,
}

impl Default for UiFile {
    fn default() -> Self {
        Self {
            language: None,
            theme: 0,
            tray_alert: true,
        }
    }
}

/// 读取一次当前的界面设置（菜单每次弹出前读取，保证与界面一致）。
pub struct UiPrefs {
    strings: HashMap<String, String>,
    pub theme: Theme,
    /// 需要留意时在托盘图标上显示圆点（设置页可关闭）。
    pub tray_alert: bool,
}

impl UiPrefs {
    pub fn load() -> Self {
        let file: UiFile = ui_file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let chinese = match file.language.as_deref() {
            Some(l) => l.starts_with("zh"),
            None => system_is_chinese(),
        };
        let strings = serde_json::from_str(if chinese { ZH } else { EN }).unwrap_or_default();
        let theme = match file.theme {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        };
        Self {
            strings,
            theme,
            tray_alert: file.tray_alert,
        }
    }

    pub fn get(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// `{0}` 形式的占位符替换。
    pub fn format(&self, key: &str, arg: &str) -> String {
        self.get(key).replace("{0}", arg)
    }
}

fn ui_file() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Poise").join("ui.json"))
}

fn system_is_chinese() -> bool {
    const LANG_CHINESE: u16 = 0x04;
    // SAFETY: 无参数。
    let lang = unsafe { GetUserDefaultUILanguage() };
    lang & 0x3FF == LANG_CHINESE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_alert_defaults_to_on_and_follows_the_file() {
        let old: UiFile = serde_json::from_str(r#"{"Language":"zh-CN","Theme":2}"#).unwrap();
        assert!(old.tray_alert);
        let off: UiFile = serde_json::from_str(r#"{"Theme":0,"TrayAlert":false}"#).unwrap();
        assert!(!off.tray_alert);
    }

    #[test]
    fn both_string_tables_have_the_tray_keys() {
        for table in [ZH, EN] {
            let map: HashMap<String, String> = serde_json::from_str(table).unwrap();
            for key in [
                "mode.auto",
                "mode.quiet",
                "mode.balanced",
                "mode.full",
                "mode.vendor.dell_g_mode",
                "home.clean",
                "tray.open",
                "tray.exit",
                "tray.boost",
                "tray.boost_none",
                "tray.tooltip",
                "status.disconnected",
            ] {
                assert!(map.contains_key(key), "缺少 {key}");
            }
        }
    }
}
