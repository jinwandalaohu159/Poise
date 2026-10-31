//! 服务数据目录的权限。服务以 SYSTEM 运行、按这里的文件做事（变更日志里的原值会被写回注册表），
//! 目录必须只有管理员能改。ProgramData 默认允许普通用户新建子目录：安装之前普通用户就可能建好
//! `ProgramData\Poise`（自己是所有者，事后还能把权限改回来）、放进伪造的文件，或者把它换成指向别处的
//! 连接点（服务写文件时就写到了别处）。所以每次启动：
//!
//! 1. 目录本身是重解析点（连接点、符号链接）：删掉链接，重建真目录；
//! 2. 所有者改为管理员组、受保护的 DACL（只有 SYSTEM、管理员可写），先只改目录本身，之后普通用户不能再往里放东西；
//! 3. 里面的重解析点：删掉链接；所有者不是 SYSTEM 或管理员组的文件：改名隔离（`*.untrusted`，不再按它
//!    做事；万一是策略让管理员建的文件归了用户，内容还在，能找回）；这样的目录删掉（留着的话它的主人
//!    还能往里放链接）；
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

/// 建好数据目录，并按上面的步骤收紧权限。返回处理了什么（要记进日志的话）：这时日志还没打开——
/// 日志文件就在这个目录里，收紧之前不能打开里面的任何文件。
pub fn secure(dir: &Path) -> io::Result<Vec<String>> {
    let mut notes = Vec::new();
    if is_reparse_point(dir) {
        notes.push(format!(
            "数据目录 {} 是指向别处的链接，删掉链接重建",
            dir.display()
        ));
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
    remove_untrusted(dir, &mut notes);
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
    status.ok().map_err(io::Error::other)?;
    Ok(notes)
}

/// 删掉目录里的重解析点和不是 SYSTEM、管理员组建的目录，这样的文件改名隔离；可信的子目录往下查。
fn remove_untrusted(dir: &Path, notes: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let handled = if is_reparse_point(&path) {
            notes.push(format!(
                "数据目录里有指向别处的链接 {}，删掉",
                path.display()
            ));
            remove_link(&path)
        } else if path.is_dir() {
            if trusted_owner(&path, notes) {
                remove_untrusted(&path, notes);
                Ok(())
            } else {
                notes.push(format!(
                    "数据目录里有不是 Poise 建的目录 {}，删掉",
                    path.display()
                ));
                std::fs::remove_dir_all(&path)
            }
        } else if is_quarantined(&path) || trusted_owner(&path, notes) {
            Ok(())
        } else {
            let aside = quarantine_path(&path);
            notes.push(format!(
                "数据目录里有不是 Poise 建的 {}，改名为 {}，不再使用",
                path.display(),
                aside.display()
            ));
            quarantine(&path, &aside)
        };
        if let Err(e) = handled {
            notes.push(format!("处理 {} 失败：{e}", path.display()));
        }
    }
}

const QUARANTINE_SUFFIX: &str = ".untrusted";

fn is_quarantined(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(&QUARANTINE_SUFFIX[1..]))
}

fn quarantine_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(QUARANTINE_SUFFIX);
    name.into()
}

/// 改名隔离（替换掉上一次留下的同名隔离文件）。
fn quarantine(path: &Path, aside: &Path) -> io::Result<()> {
    std::fs::rename(path, aside)
}

fn is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

/// 删掉链接本身（不碰它指向的东西）。
fn remove_link(path: &Path) -> io::Result<()> {
    std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
}

/// 所有者是 SYSTEM 或管理员组。读不到时按可信处理（不动用户的数据），记下来。
fn trusted_owner(path: &Path, notes: &mut Vec<String>) -> bool {
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
            notes.push(format!("读不到 {} 的所有者：{status:?}", path.display()));
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
