//! 数据目录：服务模式放在 ProgramData（只有管理员可写），开发模式放在用户目录。

use std::path::PathBuf;

use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
};

use crate::win::security::{DATA_DIR_SDDL, SecurityDescriptor};
use crate::win::wide::WideString;

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
