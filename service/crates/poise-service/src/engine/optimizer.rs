//! 系统优化项（静态 + 动态枚举），以及游戏偏好对应的系统设置。

use std::path::Path;

use poise_core::optimize::{self, ApplyReport, Hive, ItemStatus, OptimizationItem, Target, Value};
use poise_core::prefs::AppPreference;
use poise_platform::settings::dynamic_items;
use poise_platform::settings::user::UserContext;
use poise_platform::win::registry;

use poise_protocol::ErrorCode;

use super::Engine;
use super::changes::{self, GAME_PREFIX};
use super::error::RequestError;

const IFEO_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";
const GPU_PREFS_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
/// CpuPriorityClass 3 = 高。系统在游戏启动时自己应用，Poise 不需要接触游戏进程。
const IFEO_HIGH_PRIORITY: u32 = 3;

impl Engine {
    fn optimization_items(&self) -> Vec<OptimizationItem> {
        let user = self.changes.store.user();
        let mut items = optimize::static_items(wsl_installed(user.as_ref()));
        items.extend(dynamic_items::context_menu_items(user.as_ref()));
        items.extend(dynamic_items::navigation_items(user.as_ref()));
        items.extend(dynamic_items::startup_items(user.as_ref()));
        items
    }

    pub fn list_optimizations(&self) -> Vec<ItemStatus> {
        optimize::status(
            &self.optimization_items(),
            &self.changes.store,
            self.changes.journal(),
        )
    }

    pub fn apply_optimization(&mut self, id: &str) -> Result<(), RequestError> {
        let item = self
            .optimization_items()
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| RequestError::new(ErrorCode::NotFound, "没有这个优化项"))?;
        let now = self.now();
        Ok(self
            .changes
            .apply(&item.id, &item.target, &item.optimized, now)?)
    }

    pub fn revert_optimization(&mut self, id: &str) -> Result<(), RequestError> {
        if !changes::is_optimization(id) {
            return Err(RequestError::new(
                ErrorCode::NotFound,
                "这项修改不属于系统优化",
            ));
        }
        Ok(self.changes.revert(id)?)
    }

    pub fn apply_recommended(&mut self) -> Result<ApplyReport, RequestError> {
        let items = self.optimization_items();
        let now = self.now();
        Ok(self
            .changes
            .edit(|s, j| Ok(optimize::apply_recommended(&items, s, j, now)))?)
    }

    pub fn revert_optimizations(&mut self) -> Result<Vec<String>, RequestError> {
        Ok(self.changes.revert_optimizations()?)
    }

    /// 设置程序偏好。标记为游戏时，同步写入系统级的优先级预设和独显偏好。
    pub fn set_app_preference(
        &mut self,
        path: &str,
        pref: AppPreference,
    ) -> Result<(), RequestError> {
        let exe = validate_exe_path(path)?;
        self.settings.preferences.set(path, pref);
        self.save_settings();
        self.sync_game_settings(path, &exe, pref)
    }

    fn sync_game_settings(
        &mut self,
        path: &str,
        exe: &str,
        pref: AppPreference,
    ) -> Result<(), RequestError> {
        let owner = format!("{GAME_PREFIX}{}", path.to_lowercase());
        if pref != AppPreference::Game {
            if self.changes.journal().owns(&owner) {
                self.changes.revert(&owner)?;
            }
            return Ok(());
        }
        let now = self.now();
        let settings = [
            (
                Target::Registry {
                    hive: Hive::LocalMachine,
                    key: format!(r"{IFEO_KEY}\{exe}\PerfOptions"),
                    name: "CpuPriorityClass".into(),
                },
                Value::Dword(IFEO_HIGH_PRIORITY),
            ),
            (
                Target::Registry {
                    hive: Hive::CurrentUser,
                    key: GPU_PREFS_KEY.into(),
                    name: path.to_string(),
                },
                Value::String("GpuPreference=2;".into()),
            ),
        ];
        let mut first_error = None;
        for (target, value) in settings {
            if let Err(e) = self.changes.apply(&owner, &target, &value, now) {
                log::warn!("游戏偏好 {path} 写入 {target:?} 失败：{e}");
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), |e| Err(e.into()))
    }
}

/// 偏好的路径来自界面，必须是真实存在的 .exe，且文件名不含路径分隔符。
/// 它只用作键和 IFEO 子键名，服务永远不会运行它；IFEO 下只写 PerfOptions。
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

fn wsl_installed(user: Option<&UserContext>) -> bool {
    let Some(user) = user else { return false };
    let (root, prefix) = user.registry_root();
    registry::open(
        root,
        &format!(r"{prefix}Software\Microsoft\Windows\CurrentVersion\Lxss"),
        false,
    )
    .is_ok()
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
