//! 程序偏好（自动 / 始终优先 / 低优先级 / 游戏），以及标记为游戏时对应的系统设置（只有"使用独立显卡"）。

use std::path::Path;

use poise_core::prefs::AppPreference;
use poise_core::setting::{Hive, Target, Value};
use poise_protocol::ErrorCode;

use super::Engine;
use super::changes::GAME_PREFIX;
use super::error::RequestError;

const GPU_PREFS_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

impl Engine {
    /// 设置程序偏好。标记为游戏时，同步写入系统的独显偏好。
    pub fn set_app_preference(
        &mut self,
        path: &str,
        pref: AppPreference,
    ) -> Result<(), RequestError> {
        validate_exe_path(path)?;
        self.settings.preferences.set(path, pref);
        self.save_settings();
        self.sync_game_settings(path, pref)
    }

    /// 启动时按当前的偏好重新同步游戏的系统设置：旧版本写过、现在不再需要的设置
    /// （例如游戏启动时的"高"优先级预设），以及已经取消标记的游戏留下的设置都会被还原。
    pub(super) fn resync_game_settings(&mut self) {
        // 游戏的设置都按用户保存；还没人登录时（服务开机就启动）等登录后再同步。
        if self.changes.store.user().is_none() {
            return;
        }
        let mut paths: Vec<(String, AppPreference)> = self
            .settings
            .preferences
            .apps
            .iter()
            .filter(|(_, p)| **p == AppPreference::Game)
            .map(|(path, p)| (path.clone(), *p))
            .collect();
        for e in self.changes.journal().entries() {
            let Some(path) = e.owner.strip_prefix(GAME_PREFIX) else {
                continue;
            };
            if !paths.iter().any(|(p, _)| p.eq_ignore_ascii_case(path)) {
                paths.push((path.to_string(), AppPreference::Auto));
            }
        }
        for (path, pref) in paths {
            if let Err(e) = self.sync_game_settings(&path, pref) {
                log::info!("同步游戏 {path} 的系统设置失败：{}", e.detail);
            }
        }
    }

    /// 让这个程序在变更日志里的游戏设置恰好等于当前偏好需要的设置。
    fn sync_game_settings(&mut self, path: &str, pref: AppPreference) -> Result<(), RequestError> {
        let owner = format!("{GAME_PREFIX}{}", path.to_lowercase());
        let wanted = if pref == AppPreference::Game {
            game_settings(path)
        } else {
            Vec::new()
        };
        let stale = self
            .changes
            .journal()
            .entries()
            .iter()
            .any(|e| e.owner == owner && !wanted.iter().any(|(t, _)| t.same_place(&e.target)));
        if stale {
            self.changes.revert(&owner)?;
        }
        let now = self.now();
        let mut first_error = None;
        for (target, value) in wanted {
            if let Err(e) = self.changes.apply(&owner, &target, &value, now) {
                log::warn!("游戏偏好 {path} 写入 {target:?} 失败：{e}");
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), |e| Err(e.into()))
    }
}

/// 游戏需要的系统设置：使用独立显卡。不设系统级的优先级预设——提高游戏优先级会和声音、输入抢资源。
fn game_settings(path: &str) -> Vec<(Target, Value)> {
    vec![(
        Target::Registry {
            hive: Hive::CurrentUser,
            key: GPU_PREFS_KEY.into(),
            name: path.to_string(),
        },
        Value::String("GpuPreference=2;".into()),
    )]
}

/// 偏好的路径来自界面，必须是真实存在的 .exe，且文件名不含路径分隔符。
/// 它只用作键（独显偏好的值名），服务永远不会运行它。
fn validate_exe_path(path: &str) -> Result<String, RequestError> {
    let invalid =
        || RequestError::new(ErrorCode::InvalidPath, "只能为已安装的程序（.exe）设置偏好");
    let p = Path::new(path);
    let file = p.file_name().and_then(|f| f.to_str()).ok_or_else(invalid)?;
    let ok = p.is_absolute()
        && path.len() < 260
        && file.to_lowercase().ends_with(".exe")
        && !file.contains(['\\', '/', ':'])
        && p.is_file();
    if ok {
        Ok(file.to_string())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_paths_are_validated() {
        let notepad = r"C:\Windows\System32\notepad.exe";
        assert_eq!(validate_exe_path(notepad).as_deref(), Ok("notepad.exe"));
        assert!(validate_exe_path(r"C:\Windows\System32\drivers\etc\hosts").is_err());
        assert!(validate_exe_path("notepad.exe").is_err());
        assert!(validate_exe_path(r"C:\nope\missing.exe").is_err());
    }
}
