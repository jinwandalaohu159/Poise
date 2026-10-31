//! 账户与 SID：进程、会话的用户是谁（SID 的文字形式 "S-1-5-21-…"）。

use std::ffi::c_void;

use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountNameW, PSID, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SESSION, WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW,
    WTSQueryUserToken, WTSUserName,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

use super::handle::OwnedHandle;
use super::wide::WideString;

/// 当前进程用户的 SID。
pub(crate) fn current_process_sid() -> Option<String> {
    let mut token = HANDLE::default();
    // SAFETY: 当前进程的伪句柄；输出到本地句柄。
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    token_sid(&OwnedHandle::from_raw(token))
}

/// 某个会话登录用户的 SID（按会话的用户令牌）。需要 SYSTEM 权限。
pub(crate) fn session_token_sid(session_id: u32) -> Option<String> {
    let mut token = HANDLE::default();
    // SAFETY: 输出到本地句柄。
    unsafe { WTSQueryUserToken(session_id, &mut token) }.ok()?;
    token_sid(&OwnedHandle::from_raw(token))
}

/// 本进程所在会话里登录的用户的 SID：按会话的用户名查账户。
pub(crate) fn session_user_sid() -> Option<String> {
    let name = session_text(WTSUserName)?;
    let domain = session_text(WTSDomainName).unwrap_or_default();
    let account = if domain.is_empty() {
        name
    } else {
        format!(r"{domain}\{name}")
    };
    let account = WideString::new(&account);
    let (mut sid_len, mut domain_len) = (0u32, 0u32);
    let mut use_kind = SID_NAME_USE::default();
    // SAFETY: 先询问两个缓冲区的大小。
    let _ = unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            account.as_pcwstr(),
            None,
            &mut sid_len,
            None,
            &mut domain_len,
            &mut use_kind,
        )
    };
    let mut sid = vec![0u8; sid_len as usize];
    let mut domain_buf = vec![0u16; domain_len as usize];
    // SAFETY: 缓冲区按系统要求的大小分配。
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            account.as_pcwstr(),
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut sid_len,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_len,
            &mut use_kind,
        )
    }
    .ok()?;
    sid_text(PSID(sid.as_mut_ptr().cast()))
}

/// 本进程所在会话的一项文字信息（用户名、域名）。
fn session_text(class: WTS_INFO_CLASS) -> Option<String> {
    let mut buf = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: 输出由系统分配，用完用 WTSFreeMemory 释放。
    unsafe {
        WTSQuerySessionInformationW(None, WTS_CURRENT_SESSION, class, &mut buf, &mut len).ok()?;
        let text = buf.to_string().ok();
        WTSFreeMemory(buf.0.cast());
        text.filter(|t| !t.is_empty())
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
    sid_text(user.User.Sid)
}

/// SID 的文字形式（"S-1-5-21-…"）。
fn sid_text(sid: PSID) -> Option<String> {
    let mut s = PWSTR::null();
    // SAFETY: 输出字符串由系统分配，用 LocalFree 释放。
    unsafe { ConvertSidToStringSidW(sid, &mut s) }.ok()?;
    // SAFETY: s 为系统返回的 NUL 结尾字符串。
    let out = unsafe { s.to_string() }.ok();
    // SAFETY: 释放系统分配的字符串。
    unsafe {
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
    }
    out
}
