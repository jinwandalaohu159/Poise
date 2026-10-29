//! 电源计划中的设置。没写明计划时指当前计划；写的是当前计划时，要重新激活计划才生效。
//!
//! "当前计划"由调用方提供（[`super::WinSettingStore`] 在批量写入期间只查一次）。

use poise_core::setting::{StoreError, Target, Value};
use windows::core::GUID;

use super::reg_err;
use crate::settings::power_plan;

/// 计划、子组、设置的 GUID。
struct Place {
    scheme: GUID,
    subgroup: GUID,
    setting: GUID,
}

impl Place {
    fn parse(
        scheme: Option<&str>,
        subgroup: &str,
        setting: &str,
        active: impl Fn() -> Result<GUID, StoreError>,
    ) -> Result<Self, StoreError> {
        let guid = |s: &str| power_plan::parse_guid(s).ok_or(StoreError::Unsupported);
        Ok(Self {
            scheme: match scheme {
                Some(s) => guid(s)?,
                None => active()?,
            },
            subgroup: guid(subgroup)?,
            setting: guid(setting)?,
        })
    }
}

pub(super) fn read(
    scheme: Option<&str>,
    subgroup: &str,
    setting: &str,
    ac: bool,
    active: impl Fn() -> Result<GUID, StoreError>,
) -> Result<Option<Value>, StoreError> {
    let p = Place::parse(scheme, subgroup, setting, active)?;
    power_plan::read(&p.scheme, &p.subgroup, &p.setting, ac)
        .map(|v| Some(Value::Dword(v)))
        .map_err(reg_err)
}

/// 写入（不激活），返回写的是否是当前计划——是的话调用方要让计划重新生效。
pub(super) fn write(
    scheme: Option<&str>,
    subgroup: &str,
    setting: &str,
    ac: bool,
    value: &Value,
    active: impl Fn() -> Result<GUID, StoreError>,
) -> Result<bool, StoreError> {
    let Value::Dword(v) = value else {
        return Err(StoreError::Unsupported);
    };
    let p = Place::parse(scheme, subgroup, setting, &active)?;
    power_plan::write_value(&p.scheme, &p.subgroup, &p.setting, ac, *v).map_err(reg_err)?;
    Ok(active().is_ok_and(|a| a == p.scheme))
}

/// 没写明计划的位置补上当前计划。
pub(super) fn resolve(subgroup: &str, setting: &str, ac: bool, active: GUID) -> Target {
    Target::PowerSetting {
        scheme: Some(power_plan::guid_text(&active)),
        subgroup: subgroup.to_string(),
        setting: setting.to_string(),
        ac,
    }
}
