//! 目录：Windows 目录；数据目录（服务模式放在 ProgramData，只有管理员可写；开发模式放在用户目录）。

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
};

use crate::win::security::{DATA_DIR_SDDL, SecurityDescriptor};
use crate::win::wide::WideString;

/// Windows 目录（通常是 C:\Windows）。
pub fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// 路径（小写）是否在 Windows 目录下。所有"是不是 Windows 自带"的判断都用它。
pub fn is_in_windows_dir(lower_path: &str) -> bool {
    static PREFIX: OnceLock<String> = OnceLock::new();
    let prefix = PREFIX.get_or_init(|| {
        let root = windows_dir().to_string_lossy().to_lowercase();
        format!("{}\\", root.trim_end_matches('\\'))
    });
    lower_path.starts_with(prefix.as_str())
}

pub fn service_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join("Poise")
}

pub fn dev_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("Poise").join("dev")
}

/// 创建数据目录。`restrict` 为真时把权限收紧为"只有管理员可写"，
/// 防止普通用户篡改配置、诱导以 SYSTEM 运行的服务做事。
pub fn ensure_dir(dir: &PathBuf, restrict: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if restrict {
        let sd = SecurityDescriptor::from_sddl(DATA_DIR_SDDL).map_err(std::io::Error::other)?;
        let w = WideString::new(&dir.to_string_lossy());
        // SAFETY: 路径以 NUL 结尾，安全描述符在调用期间有效。
        let ok = unsafe {
            SetFileSecurityW(
                w.as_pcwstr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                sd.raw(),
            )
        };
        if !ok.as_bool() {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_dir_check_follows_the_real_windows_dir() {
        let root = windows_dir().to_string_lossy().to_lowercase();
        assert!(is_in_windows_dir(&format!(r"{root}\system32\svchost.exe")));
        // 只匹配整个目录名，"windowsapps" 这类同前缀的目录不算。
        assert!(!is_in_windows_dir(&format!(r"{root}apps\x.exe")));
        assert!(!is_in_windows_dir(r"d:\program files\app\app.exe"));
    }
}
