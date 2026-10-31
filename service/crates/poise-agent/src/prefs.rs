//! 界面的语言、外观与托盘提醒设置（界面写在 %LOCALAPPDATA%\Poise\ui.json），以及对应的文字表。
//! 文字表与界面共用同一份文件，编译时嵌入。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use poise_platform::win::handle::OwnedHandle;
use poise_platform::win::wide::WideString;
use serde::Deserialize;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

const ZH: &str = include_str!("../../../../app/Poise.Core/Localization/Strings/zh-CN.json");
const EN: &str = include_str!("../../../../app/Poise.Core/Localization/Strings/en-US.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Deserialize)]
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

/// 读取一次当前的界面设置（菜单每次弹出前读取，保证与界面一致）。文字表只解析一次，
/// 设置文件没改过就不重读。
pub struct UiPrefs {
    strings: &'static HashMap<String, String>,
    pub theme: Theme,
    /// 需要留意时在托盘图标上显示圆点（设置页可关闭）。
    pub tray_alert: bool,
}

impl UiPrefs {
    pub fn load() -> Self {
        let file = read_ui_file();
        let chinese = match file.language.as_deref() {
            Some(l) => l.starts_with("zh"),
            None => system_is_chinese(),
        };
        let strings = table(chinese);
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

/// 界面保存设置后触发的事件（见 Poise.Core 的 `Agent.PrefsChanged`）。
const PREFS_EVENT: &str = r"Local\Poise.Agent.Prefs";

/// 在后台线程里等界面保存设置，每次保存后调用 `notify`（在后台线程上调用，应只投递消息）：
/// 换了语言、关掉提醒圆点时托盘立即跟上，不等状态下次变化。
pub fn watch(notify: impl Fn() + Send + 'static) {
    let name = WideString::new(PREFS_EVENT);
    // SAFETY: 名称以 NUL 结尾；自动复位的事件，句柄交给等待线程持有。
    let Ok(event) = (unsafe { CreateEventW(None, false, false, name.as_pcwstr()) }) else {
        log::warn!("无法创建设置变化事件");
        return;
    };
    let event = OwnedHandle::from_raw(event);
    let spawned = std::thread::Builder::new()
        .name("prefs-watch".into())
        .spawn(move || {
            // SAFETY: 句柄由本线程持有，一直有效。
            while unsafe { WaitForSingleObject(event.raw(), INFINITE) } == WAIT_OBJECT_0 {
                notify();
            }
        });
    if let Err(e) = spawned {
        log::warn!("无法启动设置变化监视：{e}");
    }
}

fn ui_file() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Poise").join("ui.json"))
}

/// 界面的设置文件；修改时间没变时用上次读到的。
fn read_ui_file() -> UiFile {
    static CACHE: Mutex<Option<(SystemTime, UiFile)>> = Mutex::new(None);
    let Some(path) = ui_file() else {
        return UiFile::default();
    };
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(at), Some((cached_at, file))) = (modified, cache.as_ref())
        && at == *cached_at
    {
        return file.clone();
    }
    let file: UiFile = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *cache = modified.map(|at| (at, file.clone()));
    file
}

/// 文字表（编译时嵌入，第一次用到时解析）。
fn table(chinese: bool) -> &'static HashMap<String, String> {
    static ZH_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static EN_TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    let (cell, text) = if chinese {
        (&ZH_TABLE, ZH)
    } else {
        (&EN_TABLE, EN)
    };
    cell.get_or_init(|| serde_json::from_str(text).unwrap_or_default())
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
                "tray.update",
                "tray.update.title",
                "tray.update.body",
                "status.disconnected",
            ] {
                assert!(map.contains_key(key), "缺少 {key}");
            }
        }
    }
}
