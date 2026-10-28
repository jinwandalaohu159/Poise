//! 当前电源计划中单个设置的读写（微软公开的 Power API）。

use std::ffi::c_void;

use windows::Win32::Foundation::{HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadACValueIndex, PowerReadDCValueIndex, PowerSetActiveScheme,
    PowerWriteACValueIndex, PowerWriteDCValueIndex,
};
use windows::core::GUID;

/// 解析 "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" 形式的 GUID。
pub fn parse_guid(s: &str) -> Option<GUID> {
    let hex: String = s
        .chars()
        .filter(|c| *c != '-' && *c != '{' && *c != '}')
        .collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok().map(GUID::from_u128)
}

fn active_scheme() -> Result<GUID, WIN32_ERROR> {
    let mut ptr: *mut GUID = std::ptr::null_mut();
    // SAFETY: 输出指针由系统分配，用 LocalFree 释放。
    unsafe { PowerGetActiveScheme(None, &mut ptr) }
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
    // SAFETY: 成功时 ptr 指向有效 GUID。
    let guid = unsafe { *ptr };
    // SAFETY: 释放系统分配的内存。
    unsafe {
        let _ = LocalFree(Some(HLOCAL(ptr as *mut c_void)));
    }
    Ok(guid)
}

pub fn read(subgroup: &GUID, setting: &GUID, ac: bool) -> Result<u32, WIN32_ERROR> {
    let scheme = active_scheme()?;
    let mut value = 0u32;
    // SAFETY: 所有 GUID 指针在调用期间有效。
    let code = unsafe {
        if ac {
            PowerReadACValueIndex(
                None,
                Some(&scheme),
                Some(subgroup),
                Some(setting),
                &mut value,
            )
            .0
        } else {
            PowerReadDCValueIndex(
                None,
                Some(&scheme),
                Some(subgroup),
                Some(setting),
                &mut value,
            )
        }
    };
    WIN32_ERROR(code)
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
    Ok(value)
}

/// 写入后重新激活当前计划，让修改立即生效。
pub fn write(subgroup: &GUID, setting: &GUID, ac: bool, value: u32) -> Result<(), WIN32_ERROR> {
    write_value(subgroup, setting, ac, value)?;
    activate()
}

/// 只写入，不重新激活（连续写多项时最后调用一次 [`activate`]；重新激活一次要几百毫秒）。
pub fn write_value(
    subgroup: &GUID,
    setting: &GUID,
    ac: bool,
    value: u32,
) -> Result<(), WIN32_ERROR> {
    let scheme = active_scheme()?;
    // SAFETY: 同上。
    let code = unsafe {
        if ac {
            PowerWriteACValueIndex(None, &scheme, Some(subgroup), Some(setting), value).0
        } else {
            PowerWriteDCValueIndex(None, &scheme, Some(subgroup), Some(setting), value)
        }
    };
    WIN32_ERROR(code)
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
}

/// 重新激活当前计划，让已写入的修改生效。
pub fn activate() -> Result<(), WIN32_ERROR> {
    let scheme = active_scheme()?;
    // SAFETY: 重新激活同一个计划。
    unsafe { PowerSetActiveScheme(None, Some(&scheme)) }
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
    const PERFBOOSTMODE: &str = "be337238-0d82-4146-a960-4f3749d470c7";

    #[test]
    fn parses_guid() {
        let g = parse_guid(SUB_PROCESSOR).unwrap();
        assert_eq!(g, GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00));
        assert!(parse_guid("nope").is_none());
    }

    #[test]
    fn reads_boost_mode_without_writing() {
        let v = read(
            &parse_guid(SUB_PROCESSOR).unwrap(),
            &parse_guid(PERFBOOSTMODE).unwrap(),
            true,
        )
        .unwrap();
        assert!(v <= 6);
    }
}
