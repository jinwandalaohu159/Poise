//! 进度窗口的文字：与界面共用同一份文字表（编译时嵌入），语言由界面传入，没有时跟随系统。

use std::collections::HashMap;

use windows::Win32::Globalization::GetUserDefaultUILanguage;

const ZH: &str = include_str!("../../../../../app/Poise.Core/Localization/Strings/zh-CN.json");
const EN: &str = include_str!("../../../../../app/Poise.Core/Localization/Strings/en-US.json");

pub struct Text(HashMap<String, String>);

impl Text {
    pub fn new(lang: Option<&str>) -> Self {
        let chinese = lang.map_or_else(system_is_chinese, |l| l.starts_with("zh"));
        Self(serde_json::from_str(if chinese { ZH } else { EN }).unwrap_or_default())
    }

    pub fn get(&self, key: &str) -> String {
        self.0.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    /// `{0}`、`{1}` 形式的占位符替换。
    pub fn format(&self, key: &str, args: &[&str]) -> String {
        args.iter()
            .enumerate()
            .fold(self.get(key), |s, (i, a)| s.replace(&format!("{{{i}}}"), a))
    }
}

fn system_is_chinese() -> bool {
    const LANG_CHINESE: u16 = 0x04;
    // SAFETY: 无参数。
    let lang = unsafe { GetUserDefaultUILanguage() };
    lang & 0x3FF == LANG_CHINESE
}

/// 字节数 → "12.3 MB"。
pub fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_updater_text_exists_in_both_languages() {
        let zh: HashMap<String, String> = serde_json::from_str(ZH).unwrap();
        let en: HashMap<String, String> = serde_json::from_str(EN).unwrap();
        let keys: Vec<&String> = zh.keys().filter(|k| k.starts_with("updater.")).collect();
        assert!(keys.len() >= 10);
        for key in keys {
            assert!(en.contains_key(key), "{key}");
        }
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        let t = Text(HashMap::from([("k".to_string(), "{0} / {1}".to_string())]));
        assert_eq!(t.format("k", &["1.0 MB", "2.0 MB"]), "1.0 MB / 2.0 MB");
    }

    #[test]
    fn sizes_are_shown_in_megabytes() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(85 * 1024 * 1024 + 300 * 1024), "85.3 MB");
    }
}
