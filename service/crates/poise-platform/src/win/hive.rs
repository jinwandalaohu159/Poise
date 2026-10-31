//! 临时加载没登录的用户的配置单元（`HKEY_USERS\<SID>`）：卸载时把那个用户的修改也还原。需要管理员
//! （备份、还原特权）。用完立即卸下，用户下次登录照常加载。

use std::path::Path;

use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_BACKUP_NAME,
    SE_PRIVILEGE_ENABLED, SE_RESTORE_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Registry::{HKEY_USERS, RegLoadKeyW, RegUnLoadKeyW};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

use super::handle::OwnedHandle;
use super::wide::WideString;

/// 加载着的配置单元；丢弃时卸下。
pub struct LoadedHive {
    sid: WideString,
}

impl LoadedHive {
    /// 把用户目录里的 NTUSER.DAT 加载到 `HKEY_USERS\<sid>`。
    pub fn load(sid: &str, profile_dir: &Path) -> Result<Self, String> {
        enable(SE_BACKUP_NAME)?;
        enable(SE_RESTORE_NAME)?;
        let sid_w = WideString::new(sid);
        let file = WideString::new(&profile_dir.join("NTUSER.DAT").to_string_lossy());
        // SAFETY: 两个参数都是 NUL 结尾的字符串，调用期间有效。
        let err = unsafe { RegLoadKeyW(HKEY_USERS, sid_w.as_pcwstr(), file.as_pcwstr()) };
        if err != ERROR_SUCCESS {
            return Err(format!("加载用户 {sid} 的配置单元失败：{}", err.0));
        }
        Ok(Self { sid: sid_w })
    }
}

impl Drop for LoadedHive {
    fn drop(&mut self) {
        // SAFETY: 卸下自己加载的配置单元。
        let err = unsafe { RegUnLoadKeyW(HKEY_USERS, self.sid.as_pcwstr()) };
        if err != ERROR_SUCCESS {
            log::warn!("卸下用户配置单元失败：{}", err.0);
        }
    }
}

/// 在本进程的令牌里启用一项特权（管理员有，只是默认没启用）。
fn enable(name: PCWSTR) -> Result<(), String> {
    let mut token = HANDLE::default();
    // SAFETY: 当前进程的伪句柄；输出到本地句柄。
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    }
    .map_err(|e| format!("打开进程令牌失败：{e}"))?;
    let token = OwnedHandle::from_raw(token);
    let mut luid = LUID::default();
    // SAFETY: name 是常量字符串；输出到本地变量。
    unsafe { LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid) }
        .map_err(|e| format!("查找特权失败：{e}"))?;
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    // SAFETY: 结构在调用期间有效。
    unsafe { AdjustTokenPrivileges(token.raw(), false, Some(&privileges), 0, None, None) }
        .map_err(|e| format!("启用特权失败：{e}"))?;
    // 没有这项特权时调用也"成功"，只是没启用：交给加载时报错。
    Ok(())
}
