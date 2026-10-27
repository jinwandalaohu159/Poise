//! Windows 11 默认把新的托盘图标收进"隐藏的图标"。Poise 的状态点需要一眼看到，
//! 所以资源管理器为图标建立设置后，把它放到任务栏上一次。
//! 用户自己调整过（设置里已有 IsPromoted）就不再改动。设置按程序路径区分，
//! 所以不另做"已处理"标记：换了安装位置的图标是新图标，同样处理一次。

use poise_core::optimize::Value;
use poise_platform::win::registry;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::HKEY_CURRENT_USER;
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::GUID;

/// 资源管理器为每个托盘图标保存的设置（以程序路径区分）。
const ICON_SETTINGS: &str = r"Control Panel\NotifyIconSettings";

pub enum Outcome {
    /// 已经处理过（这次、以前，或用户自己调整过）。
    Done,
    /// 资源管理器还没为这个图标建立设置，稍后再试。
    NotYet,
}

pub fn once() -> Outcome {
    let Some(exe) = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_lowercase())
    else {
        return Outcome::Done;
    };
    let Ok(settings) = registry::open(HKEY_CURRENT_USER, ICON_SETTINGS, false) else {
        return Outcome::NotYet;
    };
    for name in settings.subkeys() {
        let path = format!(r"{ICON_SETTINGS}\{name}");
        let ours = matches!(
            registry::read_value(HKEY_CURRENT_USER, &path, "ExecutablePath"),
            Ok(Some(Value::String(p))) if expand_known_folder(&p).to_lowercase() == exe
        );
        if !ours {
            continue;
        }
        if let Ok(key) = registry::open(HKEY_CURRENT_USER, &path, true) {
            // 用户已经自己选过（有这个值）就不动。
            if matches!(key.read("IsPromoted"), Ok(None)) {
                let _ = key.write("IsPromoted", &Value::Dword(1));
                log::info!("托盘图标已放到任务栏上");
            }
        }
        return Outcome::Done;
    }
    Outcome::NotYet
}

/// 资源管理器记录的路径可能以已知文件夹的 GUID 开头
/// （例如 `{6D809377-6AF0-444B-8957-A3773F02200E}\Poise\poise-agent.exe` 表示 Program Files），
/// 换成实际路径；解析不了时原样返回。
fn expand_known_folder(path: &str) -> String {
    let Some((head, rest)) = path.split_once('\\') else {
        return path.to_string();
    };
    let Some(id) = head.strip_prefix('{').and_then(|h| h.strip_suffix('}')) else {
        return path.to_string();
    };
    let Ok(guid) = GUID::try_from(id) else {
        return path.to_string();
    };
    match known_folder(&guid) {
        Some(folder) => format!(r"{folder}\{rest}"),
        None => path.to_string(),
    }
}

fn known_folder(id: &GUID) -> Option<String> {
    // SAFETY: 返回的字符串由系统分配，读取后用 CoTaskMemFree 释放。
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_program_files_guid() {
        let expanded =
            expand_known_folder(r"{6D809377-6AF0-444B-8957-A3773F02200E}\Poise\poise-agent.exe");
        let program_files = std::env::var("ProgramW6432").unwrap();
        assert_eq!(
            expanded.to_lowercase(),
            format!(r"{program_files}\Poise\poise-agent.exe").to_lowercase()
        );
    }

    #[test]
    fn leaves_plain_and_unknown_paths_alone() {
        for p in [
            r"D:\Tools\poise-agent.exe",
            r"{not-a-guid}\poise-agent.exe",
            r"{00000000-0000-0000-0000-000000000001}\x.exe",
            "poise-agent.exe",
        ] {
            assert_eq!(expand_known_folder(p), p);
        }
    }
}
