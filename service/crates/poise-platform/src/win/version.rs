//! Windows 的版本号（内部版本号，例如 Windows 11 24H2 是 26100）。

use std::sync::OnceLock;

use poise_core::setting::Value;
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use super::registry;

/// 系统的内部版本号；读不到时为 None。只读一次。
pub fn build() -> Option<u32> {
    static BUILD: OnceLock<Option<u32>> = OnceLock::new();
    *BUILD.get_or_init(|| {
        match registry::read_value(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "CurrentBuildNumber",
        ) {
            Ok(Some(Value::String(s))) => s.trim().parse().ok(),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn this_system_has_a_build_number() {
        assert!(super::build().is_some_and(|b| b >= 10240));
    }
}
