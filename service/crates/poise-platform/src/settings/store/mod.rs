//! `SettingStore` 的 Windows 实现：按位置的种类交给各自的后端（注册表值、用户目录下的配置文件、
//! 电源计划中的设置），这里只负责分派、当前用户，以及批量写入时电源计划的重新激活；位置属于哪个用户见
//! [`owner`]。

mod owner;
mod power_setting;
mod registry_value;
mod user_file;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, RwLock};

use poise_core::setting::{SettingStore, StoreError, Target, Value};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, WIN32_ERROR};
use windows::core::GUID;

use crate::settings::power_plan;
use crate::settings::user::UserContext;

/// 当前交互用户可能变化（注销、切换用户），所以允许随时更新。
#[derive(Default)]
pub struct WinSettingStore {
    user: RwLock<Option<UserContext>>,
    /// 批量写入的嵌套层数；大于 0 时电源计划只写不激活。
    batch: AtomicU32,
    /// 批内写过当前电源计划，结束时需要重新激活一次。
    power_pending: AtomicBool,
    /// 批内查到的当前电源计划：切换模式一次要写几十项，只查一次。批结束时清空。
    batch_scheme: Mutex<Option<GUID>>,
    /// 没登录的用户的位置也读写：临时加载他的配置单元（只在卸载时还原一切用，见 [`crate::win::hive::LoadedHive`]）。
    load_absent: AtomicBool,
}

impl WinSettingStore {
    pub fn new(user: Option<UserContext>) -> Self {
        Self {
            user: RwLock::new(user),
            batch: AtomicU32::new(0),
            power_pending: AtomicBool::new(false),
            batch_scheme: Mutex::new(None),
            load_absent: AtomicBool::new(false),
        }
    }

    /// 没登录的用户的位置也读写（临时加载他的配置单元）。
    pub fn load_absent_users(&self, on: bool) {
        self.load_absent.store(on, Ordering::Release);
    }

    pub fn set_user(&self, user: Option<UserContext>) {
        if let Ok(mut u) = self.user.write() {
            *u = user;
        }
    }

    pub fn user(&self) -> Option<UserContext> {
        self.user.read().ok().and_then(|u| u.clone())
    }

    /// 当前电源计划；批内只查一次。
    fn active_scheme(&self) -> Result<GUID, StoreError> {
        let in_batch = self.batch.load(Ordering::Acquire) > 0;
        if in_batch && let Some(g) = self.batch_scheme.lock().ok().and_then(|g| *g) {
            return Ok(g);
        }
        let g = power_plan::active_scheme().map_err(reg_err)?;
        if in_batch && let Ok(mut cached) = self.batch_scheme.lock() {
            *cached = Some(g);
        }
        Ok(g)
    }

    /// 写过当前电源计划：批内推迟到结束时激活一次，否则立即激活。
    fn power_written(&self) -> Result<(), StoreError> {
        if self.batch.load(Ordering::Acquire) > 0 {
            self.power_pending.store(true, Ordering::Release);
            Ok(())
        } else {
            power_plan::activate().map_err(reg_err)
        }
    }
}

/// 注册表和电源 API 的错误码。
fn reg_err(e: WIN32_ERROR) -> StoreError {
    if e == ERROR_ACCESS_DENIED {
        StoreError::AccessDenied
    } else {
        StoreError::Io(format!("注册表错误 {}", e.0))
    }
}

impl SettingStore for WinSettingStore {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError> {
        match target {
            Target::Registry {
                hive, key, name, ..
            } => self.as_owner(target, |user| registry_value::read(user, *hive, key, name)),
            Target::Ini {
                file, section, key, ..
            } => self.as_owner(target, |user| user_file::read(user, *file, section, key)),
            Target::PowerSetting {
                scheme,
                subgroup,
                setting,
                ac,
            } => power_setting::read(scheme.as_deref(), subgroup, setting, *ac, || {
                self.active_scheme()
            }),
        }
    }

    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError> {
        match target {
            Target::Registry {
                hive, key, name, ..
            } => self.as_owner(target, |user| {
                registry_value::write(user, *hive, key, name, value)
            }),
            Target::Ini {
                file, section, key, ..
            } => self.as_owner(target, |user| {
                user_file::write(user, *file, section, key, value)
            }),
            Target::PowerSetting {
                scheme,
                subgroup,
                setting,
                ac,
            } => {
                let active = || self.active_scheme();
                if power_setting::write(scheme.as_deref(), subgroup, setting, *ac, value, active)? {
                    self.power_written()?;
                }
                Ok(())
            }
        }
    }

    fn delete(&self, target: &Target) -> Result<(), StoreError> {
        match target {
            Target::Registry {
                hive, key, name, ..
            } => self.as_owner(target, |user| {
                registry_value::delete(user, *hive, key, name)
            }),
            Target::Ini {
                file, section, key, ..
            } => self.as_owner(target, |user| user_file::delete(user, *file, section, key)),
            // 电源设置总是存在，还原时写回原值，不会走到删除。
            Target::PowerSetting { .. } => Err(StoreError::Unsupported),
        }
    }

    /// 电源设置没写明计划时，补上当前计划；按用户的位置没写明用户时，补上当前用户（知道是谁时；控制台方式
    /// 下就是运行 Poise 的用户，不写明）。
    fn resolve(&self, target: &Target) -> Result<Target, StoreError> {
        if target.per_user() && target.user().is_none() {
            let sid = self.user().and_then(|u| u.sid);
            return Ok(match sid {
                Some(sid) => target.clone().for_user(&sid),
                None => target.clone(),
            });
        }
        match target {
            Target::PowerSetting {
                scheme: None,
                subgroup,
                setting,
                ac,
            } => Ok(power_setting::resolve(
                subgroup,
                setting,
                *ac,
                self.active_scheme()?,
            )),
            _ => Ok(target.clone()),
        }
    }

    fn begin_batch(&self) {
        self.batch.fetch_add(1, Ordering::AcqRel);
    }

    fn end_batch(&self) {
        if self.end_batch_deferred()
            && let Err(e) = power_plan::activate()
        {
            log::warn!("重新激活电源计划失败：{e:?}");
        }
    }

    fn end_batch_deferred(&self) -> bool {
        let outermost = self.batch.fetch_sub(1, Ordering::AcqRel) == 1;
        if outermost && let Ok(mut cached) = self.batch_scheme.lock() {
            *cached = None;
        }
        outermost && self.power_pending.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use poise_core::journal::Journal;
    use poise_core::setting::{Hive, UserFile};
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
        let target = Target::registry(Hive::CurrentUser, SCRATCH, "Value");
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
        let target = Target::ini(UserFile::WslConfig, "experimental", "autoMemoryReclaim");
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
    fn power_settings_resolve_to_the_active_scheme() {
        let store = WinSettingStore::new(None);
        let relative = Target::power(
            "54533251-82be-4824-96c1-47b60b740d00",
            "be337238-0d82-4146-a960-4f3749d470c7",
            true,
        );
        let resolved = store.resolve(&relative).unwrap();
        let Target::PowerSetting {
            scheme: Some(scheme),
            ..
        } = &resolved
        else {
            panic!("没有补上计划：{resolved:?}");
        };
        assert_eq!(
            power_plan::parse_guid(scheme),
            power_plan::active_scheme().ok()
        );
        // 只读不写：两种写法读到的是同一个值，但记成两个不同的位置。
        assert_eq!(store.read(&relative), store.read(&resolved));
        assert!(!relative.same_place(&resolved));
    }

    #[test]
    fn current_user_targets_need_a_user() {
        let store = WinSettingStore::new(None);
        let target = Target::registry(Hive::CurrentUser, SCRATCH, "x");
        assert_eq!(store.read(&target), Err(StoreError::Unsupported));
    }
}
