//! 位置属于哪个用户：没写明的是当前交互用户；写明了 SID 的就是那个用户——他不是当前用户时现找，没登录时
//! 不读写他的注册表（卸载时临时加载他的配置单元，见 [`LoadedHive`]）。

use std::sync::atomic::Ordering;

use poise_core::setting::{StoreError, Target};

use super::WinSettingStore;
use crate::settings::user::UserContext;
use crate::win::hive::LoadedHive;

impl WinSettingStore {
    /// 在位置所属的用户那里读写：写明了 SID 的就是那个用户（不是当前用户时现找），否则是当前用户。
    /// 他没登录（配置单元没加载）时不读写注册表：免得把"没加载"当成"值不存在"；卸载时临时加载。
    pub(super) fn as_owner<T>(
        &self,
        target: &Target,
        f: impl FnOnce(Option<&UserContext>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let current = self.user();
        let Some(sid) = target.user() else {
            return f(current.as_ref());
        };
        let is_current = current
            .as_ref()
            .and_then(|u| u.sid.as_deref())
            .is_some_and(|s| s.eq_ignore_ascii_case(sid));
        if is_current {
            return f(current.as_ref());
        }
        let other = UserContext::for_sid(sid.to_string())
            .ok_or_else(|| StoreError::Io(format!("找不到用户 {sid}")))?;
        let _hive = if matches!(target, Target::Registry { .. }) && !other.hive_loaded() {
            if !self.load_absent.load(Ordering::Acquire) {
                return Err(StoreError::Io(format!(
                    "用户 {sid} 没有登录，等他登录后再改"
                )));
            }
            Some(LoadedHive::load(sid, &other.profile_dir).map_err(StoreError::Io)?)
        } else {
            None
        };
        f(Some(&other))
    }
}
