//! `SettingStore` 的 Windows 实现：注册表、用户目录下的 INI 文件、电源计划。

use std::path::PathBuf;
use std::sync::RwLock;

use poise_core::optimize::{Hive, SettingStore, StoreError, Target, UserFile, Value};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, WIN32_ERROR};
use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE};

use crate::settings::user::UserContext;
use crate::settings::{ini, power_plan};
use crate::win::registry;

/// 当前交互用户可能变化（注销、切换用户），所以允许随时更新。
#[derive(Default)]
pub struct WinSettingStore {
    user: RwLock<Option<UserContext>>,
}

impl WinSettingStore {
    pub fn new(user: Option<UserContext>) -> Self {
        Self {
            user: RwLock::new(user),
        }
    }

    pub fn set_user(&self, user: Option<UserContext>) {
        if let Ok(mut u) = self.user.write() {
            *u = user;
        }
    }

    pub fn user(&self) -> Option<UserContext> {
        self.user.read().ok().and_then(|u| u.clone())
    }

    fn registry_location(&self, hive: Hive, key: &str) -> Result<(HKEY, String), StoreError> {
        match hive {
            Hive::LocalMachine => Ok((HKEY_LOCAL_MACHINE, key.to_string())),
            Hive::CurrentUser => {
                let user = self.user().ok_or(StoreError::Unsupported)?;
                let (root, prefix) = user.registry_root();
                Ok((root, format!("{prefix}{key}")))
            }
        }
    }

    fn user_file(&self, file: UserFile) -> Result<PathBuf, StoreError> {
        let user = self.user().ok_or(StoreError::Unsupported)?;
        Ok(match file {
            UserFile::WslConfig => user.profile_dir.join(".wslconfig"),
        })
    }
}

fn reg_err(e: WIN32_ERROR) -> StoreError {
    if e == ERROR_ACCESS_DENIED {
        StoreError::AccessDenied
    } else {
        StoreError::Io(format!("注册表错误 {}", e.0))
    }
}

fn io_err(e: std::io::Error) -> StoreError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        StoreError::AccessDenied
    } else {
        StoreError::Io(e.to_string())
    }
}

fn read_text(path: &PathBuf) -> Result<String, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_err(e)),
    }
}

fn power_guids(
    subgroup: &str,
    setting: &str,
) -> Result<(windows::core::GUID, windows::core::GUID), StoreError> {
    Ok((
        power_plan::parse_guid(subgroup).ok_or(StoreError::Unsupported)?,
        power_plan::parse_guid(setting).ok_or(StoreError::Unsupported)?,
    ))
}

impl SettingStore for WinSettingStore {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError> {
        match target {
            Target::Registry { hive, key, name } => {
                let (root, path) = self.registry_location(*hive, key)?;
                registry::read_value(root, &path, name).map_err(reg_err)
            }
            Target::Ini { file, section, key } => {
                let text = read_text(&self.user_file(*file)?)?;
                Ok(ini::get(&text, section, key).map(Value::String))
            }
            Target::PowerSetting {
                subgroup,
                setting,
                ac,
            } => {
                let (sub, set) = power_guids(subgroup, setting)?;
                power_plan::read(&sub, &set, *ac)
                    .map(|v| Some(Value::Dword(v)))
                    .map_err(reg_err)
            }
        }
    }

    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError> {
        match target {
            Target::Registry { hive, key, name } => {
                let (root, path) = self.registry_location(*hive, key)?;
                registry::create(root, &path)
                    .and_then(|k| k.write(name, value))
                    .map_err(reg_err)
            }
            Target::Ini { file, section, key } => {
                let Value::String(v) = value else {
                    return Err(StoreError::Unsupported);
                };
                let path = self.user_file(*file)?;
                let text = read_text(&path)?;
                std::fs::write(&path, ini::set(&text, section, key, v)).map_err(io_err)
            }
            Target::PowerSetting {
                subgroup,
                setting,
                ac,
            } => {
                let Value::Dword(v) = value else {
                    return Err(StoreError::Unsupported);
                };
                let (sub, set) = power_guids(subgroup, setting)?;
                power_plan::write(&sub, &set, *ac, *v).map_err(reg_err)
            }
        }
    }

    fn delete(&self, target: &Target) -> Result<(), StoreError> {
        match target {
            Target::Registry { hive, key, name } => {
                let (root, path) = self.registry_location(*hive, key)?;
                match registry::open(root, &path, true) {
                    Ok(k) => k.delete(name).map_err(reg_err),
                    // 键都不存在，值自然也不存在。
                    Err(e) if e == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND => Ok(()),
                    Err(e) => Err(reg_err(e)),
                }
            }
            Target::Ini { file, section, key } => {
                let path = self.user_file(*file)?;
                let text = read_text(&path)?;
                if text.is_empty() {
                    return Ok(());
                }
                std::fs::write(&path, ini::remove(&text, section, key)).map_err(io_err)
            }
            // 电源设置总是存在，还原时写回原值，不会走到删除。
            Target::PowerSetting { .. } => Err(StoreError::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use poise_core::journal::Journal;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};

    use super::*;
    use crate::win::wide::WideString;

    const SCRATCH: &str = r"Software\PoiseSelfTest";

    /// 测试结束后删除整个临时键。
    struct Scratch;
    impl Drop for Scratch {
        fn drop(&mut self) {
            let w = WideString::new(SCRATCH);
            // SAFETY: 只删除测试专用的键。
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, w.as_pcwstr());
            }
        }
    }

    fn store_in(dir: PathBuf) -> WinSettingStore {
        WinSettingStore::new(Some(UserContext {
            sid: None,
            profile_dir: dir,
        }))
    }

    #[test]
    fn journal_round_trip_on_real_registry() {
        let _guard = Scratch;
        let store = store_in(std::env::temp_dir());
        let target = Target::Registry {
            hive: Hive::CurrentUser,
            key: SCRATCH.into(),
            name: "Value".into(),
        };
        store.write(&target, &Value::Dword(7)).unwrap();
        let mut j = Journal::default();
        j.apply(&store, "t", &target, &Value::Dword(1), 0).unwrap();
        assert_eq!(store.read(&target).unwrap(), Some(Value::Dword(1)));
        j.revert(&store, "t").unwrap();
        assert_eq!(store.read(&target).unwrap(), Some(Value::Dword(7)));
    }

    #[test]
    fn ini_round_trip_in_temp_profile() {
        let dir = std::env::temp_dir().join(format!("poise-ini-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".wslconfig"), "[wsl2]\r\nmemory=8GB\r\n").unwrap();
        let store = store_in(dir.clone());
        let target = Target::Ini {
            file: UserFile::WslConfig,
            section: "experimental".into(),
            key: "autoMemoryReclaim".into(),
        };
        let mut j = Journal::default();
        j.apply(&store, "wsl", &target, &Value::String("gradual".into()), 0)
            .unwrap();
        assert_eq!(
            store.read(&target).unwrap(),
            Some(Value::String("gradual".into()))
        );
        j.revert(&store, "wsl").unwrap();
        assert_eq!(store.read(&target).unwrap(), None);
        let text = std::fs::read_to_string(dir.join(".wslconfig")).unwrap();
        assert!(text.contains("memory=8GB"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn current_user_targets_need_a_user() {
        let store = WinSettingStore::new(None);
        let target = Target::Registry {
            hive: Hive::CurrentUser,
            key: SCRATCH.into(),
            name: "x".into(),
        };
        assert_eq!(store.read(&target), Err(StoreError::Unsupported));
    }
}
