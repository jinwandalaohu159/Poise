//! 服务数据目录的权限。服务以 SYSTEM 运行、按这里的文件做事（变更日志里的原值会被写回注册表），
//! 目录必须只有管理员能改。ProgramData 默认允许普通用户新建子目录：安装之前普通用户就可能建好
//! `ProgramData\Poise`（自己是所有者，事后还能把权限改回来）、放进伪造的文件，或者把它换成指向别处的
//! 连接点（服务写文件时就写到了别处）。所以每次启动：
//!
//! 1. 目录本身是重解析点（连接点、符号链接）：删掉链接，重建真目录；
//! 2. 所有者改为管理员组、受保护的 DACL（只有 SYSTEM、管理员可写），先只改目录本身，之后普通用户不能再往里放东西；
//! 3. 里面的重解析点，以及所有者不是 SYSTEM 或管理员组的文件和目录：删掉（不是 Poise 写的）；
//! 4. 剩下的按目录的权限重新继承（这时里面已经没有重解析点，不会改到别处的权限）。

use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::Path;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
    IsWellKnownSid, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SetFileSecurityW, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use crate::win::security::{DATA_DIR_SDDL, SecurityDescriptor};
use crate::win::wide::WideString;

/// 建好数据目录，并按上面的步骤收紧权限。
pub fn secure(dir: &Path) -> io::Result<()> {
    if is_reparse_point(dir) {
        log::warn!("数据目录 {} 是指向别处的链接，删掉链接重建", dir.display());
        remove_link(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    let sd = SecurityDescriptor::from_sddl(DATA_DIR_SDDL).map_err(io::Error::other)?;
    let w = WideString::new(&dir.to_string_lossy());
    let info = OWNER_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION;
    // SAFETY: 路径以 NUL 结尾，安全描述符在调用期间有效。
    if !unsafe { SetFileSecurityW(w.as_pcwstr(), info, sd.raw()) }.as_bool() {
        return Err(io::Error::last_os_error());
    }
    remove_untrusted(dir);
    let (owner, dacl) = owner_and_dacl(&sd)?;
    // SAFETY: 所有者和 DACL 指向 `sd` 内部，调用期间有效。
    let status = unsafe {
        SetNamedSecurityInfoW(
            w.as_pcwstr(),
            SE_FILE_OBJECT,
            info,
            Some(owner),
            None,
            Some(dacl),
            None,
        )
    };
    status.ok().map_err(io::Error::other)
}

/// 删掉目录里的重解析点和不是 SYSTEM、管理员组建的东西；可信的子目录往下查。
fn remove_untrusted(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let removed = if is_reparse_point(&path) {
            log::warn!("数据目录里有指向别处的链接 {}，删掉", path.display());
            remove_link(&path)
        } else if !trusted_owner(&path) {
            log::warn!("数据目录里有不是 Poise 建的 {}，删掉", path.display());
            if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            }
        } else {
            if path.is_dir() {
                remove_untrusted(&path);
            }
            Ok(())
        };
        if let Err(e) = removed {
            log::warn!("删除 {} 失败：{e}", path.display());
        }
    }
}

fn is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

/// 删掉链接本身（不碰它指向的东西）。
fn remove_link(path: &Path) -> io::Result<()> {
    std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
}

/// 所有者是 SYSTEM 或管理员组。读不到时按可信处理（不删用户的数据），记日志。
fn trusted_owner(path: &Path) -> bool {
    let w = WideString::new(&path.to_string_lossy());
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: 输出由系统分配，用完用 LocalFree 释放；所有者指向 `sd` 内部。
    unsafe {
        let status = GetNamedSecurityInfoW(
            w.as_pcwstr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        );
        if status.is_err() {
            log::warn!("读不到 {} 的所有者：{status:?}", path.display());
            return true;
        }
        let trusted = IsWellKnownSid(owner, WinLocalSystemSid).as_bool()
            || IsWellKnownSid(owner, WinBuiltinAdministratorsSid).as_bool();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        trusted
    }
}

/// 安全描述符里的所有者与 DACL（指向描述符内部）。
fn owner_and_dacl(sd: &SecurityDescriptor) -> io::Result<(PSID, *const ACL)> {
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let (mut defaulted, mut present) = (Default::default(), Default::default());
    // SAFETY: 描述符有效；输出指向它内部。
    unsafe {
        GetSecurityDescriptorOwner(sd.raw(), &mut owner, &mut defaulted)
            .map_err(io::Error::other)?;
        GetSecurityDescriptorDacl(sd.raw(), &mut present, &mut dacl, &mut defaulted)
            .map_err(io::Error::other)?;
    }
    Ok((owner, dacl))
}
