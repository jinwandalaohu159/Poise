//! 注册表值：本机或当前交互用户的配置单元。

use poise_core::setting::{Hive, StoreError, Value};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE};

use super::reg_err;
use crate::settings::user::UserContext;
use crate::win::registry;

/// 注册表位置。当前用户的键在服务以 SYSTEM 运行时解析到该用户的配置单元；还没有用户时不支持。
fn location(
    user: Option<&UserContext>,
    hive: Hive,
    key: &str,
) -> Result<(HKEY, String), StoreError> {
    match hive {
        Hive::LocalMachine => Ok((HKEY_LOCAL_MACHINE, key.to_string())),
        Hive::CurrentUser => {
            let (root, prefix) = user.ok_or(StoreError::Unsupported)?.registry_root();
            Ok((root, format!("{prefix}{key}")))
        }
    }
}

pub(super) fn read(
    user: Option<&UserContext>,
    hive: Hive,
    key: &str,
    name: &str,
) -> Result<Option<Value>, StoreError> {
    let (root, path) = location(user, hive, key)?;
    registry::read_value(root, &path, name).map_err(reg_err)
}

pub(super) fn write(
    user: Option<&UserContext>,
    hive: Hive,
    key: &str,
    name: &str,
    value: &Value,
) -> Result<(), StoreError> {
    let (root, path) = location(user, hive, key)?;
    registry::create(root, &path)
        .and_then(|k| k.write(name, value))
        .map_err(reg_err)
}

pub(super) fn delete(
    user: Option<&UserContext>,
    hive: Hive,
    key: &str,
    name: &str,
) -> Result<(), StoreError> {
    let (root, path) = location(user, hive, key)?;
    match registry::open(root, &path, true) {
        Ok(k) => k.delete(name).map_err(reg_err),
        // 键都不存在，值自然也不存在。
        Err(e) if e == ERROR_FILE_NOT_FOUND => Ok(()),
        Err(e) => Err(reg_err(e)),
    }
}
