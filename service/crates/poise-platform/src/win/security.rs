//! 安全描述符：数据目录只有管理员可写，命名管道只有本机交互用户可连。

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

use crate::win::wide::WideString;

/// SYSTEM、管理员完全控制；普通用户只读。受保护的 DACL，不继承上级权限。
pub const DATA_DIR_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// SYSTEM、管理员完全控制；本机交互登录用户可读写。远程连接另由管道标志拒绝。
pub const SERVICE_PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

/// 由 SDDL 生成的安全描述符，离开作用域时释放。
pub struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    pub fn from_sddl(sddl: &str) -> windows::core::Result<Self> {
        let w = WideString::new(sddl);
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: 输出由系统分配，在 Drop 中用 LocalFree 释放。
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w.as_pcwstr(),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }?;
        Ok(Self(sd))
    }

    pub fn raw(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }

    /// 生成引用本描述符的 SECURITY_ATTRIBUTES（生命周期不得超过 self）。
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: 由 ConvertStringSecurityDescriptorToSecurityDescriptorW 分配。
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sddl_strings_parse() {
        assert!(SecurityDescriptor::from_sddl(DATA_DIR_SDDL).is_ok());
        assert!(SecurityDescriptor::from_sddl(SERVICE_PIPE_SDDL).is_ok());
    }
}
