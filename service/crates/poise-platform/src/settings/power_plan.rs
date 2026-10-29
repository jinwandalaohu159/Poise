//! 电源计划中单个设置的读写（微软公开的 Power API），以及 Windows 自带的出厂值和电源模式滑块。

use std::ffi::c_void;

use windows::Win32::Foundation::{HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadACDefaultIndex, PowerReadACValueIndex, PowerReadDCDefaultIndex,
    PowerReadDCValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex, PowerWriteDCValueIndex,
};
use windows::core::GUID;

/// Windows"平衡"计划（也是计划的"个性"之一：OEM 自定义的计划多数以它为基础）。
pub const BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);

/// 电源模式滑块的"最佳能效"：它会盖住计划里的能效偏好、最小处理器状态等设置。
pub const OVERLAY_BETTER_BATTERY: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);

#[link(name = "powrprof", kind = "raw-dylib")]
unsafe extern "system" {
    /// 当前实际生效的电源模式滑块（全零表示没有覆盖，即"平衡"）。Windows 10 1709 起提供，未写入 SDK 头文件。
    fn PowerGetEffectiveOverlayScheme(overlay: *mut GUID) -> u32;
}

fn check(code: u32) -> Result<(), WIN32_ERROR> {
    WIN32_ERROR(code)
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
}

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

/// GUID 写成 "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"（小写）。
pub fn guid_text(g: &GUID) -> String {
    format!("{g:?}").to_lowercase()
}

/// 当前生效的电源计划。
pub fn active_scheme() -> Result<GUID, WIN32_ERROR> {
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

pub fn read(scheme: &GUID, subgroup: &GUID, setting: &GUID, ac: bool) -> Result<u32, WIN32_ERROR> {
    let mut value = 0u32;
    // SAFETY: 所有 GUID 指针在调用期间有效。
    let code = unsafe {
        if ac {
            PowerReadACValueIndex(
                None,
                Some(scheme),
                Some(subgroup),
                Some(setting),
                &mut value,
            )
            .0
        } else {
            PowerReadDCValueIndex(
                None,
                Some(scheme),
                Some(subgroup),
                Some(setting),
                &mut value,
            )
        }
    };
    check(code)?;
    Ok(value)
}

/// 只写入，不重新激活。写的是当前计划时，要调用 [`activate`] 才生效（连续写多项时最后调用一次；
/// 重新激活一次要几百毫秒）；写别的计划不需要激活，切换到那个计划时自然生效。
pub fn write_value(
    scheme: &GUID,
    subgroup: &GUID,
    setting: &GUID,
    ac: bool,
    value: u32,
) -> Result<(), WIN32_ERROR> {
    // SAFETY: 同上。
    let code = unsafe {
        if ac {
            PowerWriteACValueIndex(None, scheme, Some(subgroup), Some(setting), value).0
        } else {
            PowerWriteDCValueIndex(None, scheme, Some(subgroup), Some(setting), value)
        }
    };
    check(code)
}

/// 重新激活当前计划，让已写入的修改生效。
pub fn activate() -> Result<(), WIN32_ERROR> {
    let scheme = active_scheme()?;
    // SAFETY: 重新激活同一个计划。
    unsafe { PowerSetActiveScheme(None, Some(&scheme)) }
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
}

/// 这台电脑上 Windows 给某种计划（个性）的出厂值：不受任何程序修改计划的影响。
pub fn default_value(
    personality: &GUID,
    subgroup: &GUID,
    setting: &GUID,
    ac: bool,
) -> Result<u32, WIN32_ERROR> {
    let mut value = 0u32;
    // SAFETY: 所有 GUID 指针在调用期间有效。
    let code = unsafe {
        if ac {
            PowerReadACDefaultIndex(None, personality, Some(subgroup), setting, &mut value)
        } else {
            PowerReadDCDefaultIndex(None, personality, Some(subgroup), setting, &mut value)
        }
    };
    check(code)?;
    Ok(value)
}

/// 当前生效的电源模式滑块；没有覆盖时为 None。
pub fn effective_overlay() -> Option<GUID> {
    let mut guid = GUID::zeroed();
    // SAFETY: 输出到本地变量。
    let code = unsafe { PowerGetEffectiveOverlayScheme(&mut guid) };
    (code == 0 && guid != GUID::zeroed()).then_some(guid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
    const PERFBOOSTMODE: &str = "be337238-0d82-4146-a960-4f3749d470c7";
    const PROCTHROTTLEMIN: &str = "893dee8e-2bef-41e0-89c6-b55d0929964c";

    #[test]
    fn parses_and_prints_guids() {
        let g = parse_guid(SUB_PROCESSOR).unwrap();
        assert_eq!(g, GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00));
        assert_eq!(guid_text(&g), SUB_PROCESSOR);
        assert!(parse_guid("nope").is_none());
    }

    #[test]
    fn reads_boost_mode_without_writing() {
        let v = read(
            &active_scheme().unwrap(),
            &parse_guid(SUB_PROCESSOR).unwrap(),
            &parse_guid(PERFBOOSTMODE).unwrap(),
            true,
        )
        .unwrap();
        assert!(v <= 6);
    }

    #[test]
    fn reads_the_windows_balanced_defaults() {
        let sub = parse_guid(SUB_PROCESSOR).unwrap();
        let min = default_value(&BALANCED, &sub, &parse_guid(PROCTHROTTLEMIN).unwrap(), true);
        // 平衡计划插电时的最小处理器状态，Windows 出厂是 5%（OEM 可能改，但不会是 100%）。
        assert!(min.unwrap() < 100);
        let boost = default_value(&BALANCED, &sub, &parse_guid(PERFBOOSTMODE).unwrap(), true);
        assert!(boost.unwrap() > 0);
    }

    #[test]
    fn reads_the_overlay_without_failing() {
        let _ = effective_overlay();
    }
}
