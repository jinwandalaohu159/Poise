//! 界面的语言与外观设置（界面写在 %LOCALAPPDATA%\Poise\ui.json），以及对应的文字表。
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

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct UiFile {
    language: Option<String>,
    #[serde(default)]
    theme: u8,
}

/// 读取一次当前的界面设置（菜单每次弹出前读取，保证与界面一致）。
pub struct UiPrefs {
    strings: HashMap<String, String>,
    pub theme: Theme,
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
        Self { strings, theme }
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
    fn both_string_tables_have_the_tray_keys() {
        for table in [ZH, EN] {
            let map: HashMap<String, String> = serde_json::from_str(table).unwrap();
            for key in [
                "mode.quiet",
                "mode.balanced",
                "mode.full",
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
