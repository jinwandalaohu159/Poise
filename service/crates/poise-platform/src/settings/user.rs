//! "当前用户"在注册表和文件系统中的位置。
//!
//! 控制台模式：就是运行 Poise 的用户（HKEY_CURRENT_USER）。
//! 服务模式：服务以 SYSTEM 运行，需要按会话找到登录用户，写入 HKEY_USERS\<SID>。

use std::ffi::c_void;
use std::path::PathBuf;

use poise_core::optimize::Value;
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS};
use windows::Win32::System::RemoteDesktop::WTSQueryUserToken;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

use crate::win::handle::OwnedHandle;
use crate::win::registry;

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
        let mut token = HANDLE::default();
        // SAFETY: 当前进程的伪句柄；输出到本地句柄。
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
        token_sid(&OwnedHandle::from_raw(token))
    }

    /// 服务模式下解析某个会话的登录用户。需要 SYSTEM 权限。
    pub fn for_session(session_id: u32) -> Option<Self> {
        let mut token = HANDLE::default();
        // SAFETY: 输出到本地句柄。
        unsafe { WTSQueryUserToken(session_id, &mut token) }.ok()?;
        let token = OwnedHandle::from_raw(token);
        let sid = token_sid(&token)?;
        let profile = match registry::read_value(
            HKEY_LOCAL_MACHINE,
            &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"),
            "ProfileImagePath",
        ) {
            Ok(Some(Value::String(p))) => PathBuf::from(p),
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

fn token_sid(token: &OwnedHandle) -> Option<String> {
    let mut len = 0u32;
    // SAFETY: 先询问大小。
    let _ = unsafe { GetTokenInformation(token.raw(), TokenUser, None, 0, &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节。
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        )
    }
    .ok()?;
    // SAFETY: 缓冲区开头是 TOKEN_USER。
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut s = PWSTR::null();
    // SAFETY: 输出字符串由系统分配，用 LocalFree 释放。
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut s) }.ok()?;
    // SAFETY: s 为系统返回的 NUL 结尾字符串。
    let out = unsafe { s.to_string() }.ok();
    // SAFETY: 释放系统分配的字符串。
    unsafe {
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
    }
    out
}
