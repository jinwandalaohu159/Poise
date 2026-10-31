//! "当前用户"在注册表和文件系统中的位置。
//!
//! 控制台模式：就是运行 Poise 的用户（HKEY_CURRENT_USER）。
//! 服务模式：服务以 SYSTEM 运行，需要按会话找到登录用户，写入 HKEY_USERS\<SID>。

use std::path::PathBuf;

use poise_core::setting::Value;
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS};

use crate::win::{account, registry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserContext {
    /// None 表示就是当前进程的用户（HKEY_CURRENT_USER）。
    pub sid: Option<String>,
    pub profile_dir: PathBuf,
}

impl UserContext {
    pub fn current_process_user() -> Option<Self> {
        let profile = std::env::var_os("USERPROFILE")?;
        Some(Self {
            sid: None,
            profile_dir: PathBuf::from(profile),
        })
    }

    /// 当前进程用户的 SID（例如 "S-1-5-21-…"）。
    pub fn current_process_sid() -> Option<String> {
        account::current_process_sid()
    }

    /// 服务模式下解析某个会话的登录用户。需要 SYSTEM 权限。
    pub fn for_session(session_id: u32) -> Option<Self> {
        Self::for_sid(account::session_token_sid(session_id)?)
    }

    /// 本进程所在会话里登录的用户（不是本进程的用户）。普通用户输入管理员的密码运行安装程序时，安装程序
    /// 以管理员身份运行，但要开启开机启动、还原修改的是正在用电脑的这个用户。
    pub fn session_user() -> Option<Self> {
        Self::for_sid(account::session_user_sid()?)
    }

    /// 按 SID 找用户目录（用户的配置单元在 `HKEY_USERS\<SID>`，登录着才加载）。
    pub fn for_sid(sid: String) -> Option<Self> {
        let profile = match registry::read_value(
            HKEY_LOCAL_MACHINE,
            &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"),
            "ProfileImagePath",
        ) {
            // 通常是 REG_EXPAND_SZ（`%SystemDrive%\Users\x`）。
            Ok(Some(Value::String(p))) => PathBuf::from(crate::win::env::expand(&p)),
            _ => return None,
        };
        Some(Self {
            sid: Some(sid),
            profile_dir: profile,
        })
    }

    /// 用户配置单元的根和路径前缀。
    pub fn registry_root(&self) -> (HKEY, String) {
        match &self.sid {
            None => (HKEY_CURRENT_USER, String::new()),
            Some(sid) => (HKEY_USERS, format!(r"{sid}\")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_user_is_found_by_account_name() {
        // 测试以登录用户本人运行：会话里的用户就是本进程的用户。
        let found = UserContext::session_user().and_then(|u| u.sid);
        assert_eq!(found, UserContext::current_process_sid());
    }
}
