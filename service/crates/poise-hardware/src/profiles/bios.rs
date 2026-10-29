//! 从注册表读取主板/整机型号（不依赖 WMI）。

use poise_core::setting::Value;
use poise_platform::win::registry;
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BiosInfo {
    pub manufacturer: String,
    pub product: String,
}

pub fn read_bios() -> BiosInfo {
    let read = |name| match registry::read_value(
        HKEY_LOCAL_MACHINE,
        r"HARDWARE\DESCRIPTION\System\BIOS",
        name,
    ) {
        Ok(Some(Value::String(s))) => s.trim().to_string(),
        _ => String::new(),
    };
    BiosInfo {
        manufacturer: read("SystemManufacturer"),
        product: read("SystemProductName"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_this_machine() {
        let b = super::read_bios();
        assert!(!b.manufacturer.is_empty());
    }
}
