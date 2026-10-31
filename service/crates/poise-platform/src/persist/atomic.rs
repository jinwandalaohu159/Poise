//! 可靠地读写一个文件：先写临时文件、落盘、再替换（替换本身也写透到磁盘），一时失败（杀毒软件、备份程序
//! 正独占着文件）时隔一会儿再试；解析不了的文件带上时间改名保留。JSON 文件（[`super::storage`]）、
//! 变更日志（[`super::journal_file`]）、后台写文件（[`super::writer`]）共用。

use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::Duration;

use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

use crate::win::wide::WideString;

/// 读写一时失败（杀毒软件、备份程序正独占着文件）时，隔这么久再试，最多试 [`ATTEMPTS`] 次。
const RETRY_AFTER: Duration = Duration::from_millis(100);
const ATTEMPTS: u32 = 5;

/// 读文件；不存在时为 None。一时读不到（被独占）时隔一会儿再试。
pub(super) fn read_retrying(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    retrying(|| match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    })
}

fn retrying<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 1;
    loop {
        match op() {
            Err(_) if attempt < ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(RETRY_AFTER);
            }
            done => return done,
        }
    }
}

/// 解析不了的文件改名保留（带上时间，不覆盖之前留下的），便于排查和找回。
pub(super) fn set_aside(path: &Path, why: &str) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let aside = path.with_extension(format!("{secs}.corrupt"));
    log::warn!(
        "{} 无法解析（{why}），已改名为 {}",
        path.display(),
        aside.display()
    );
    let _ = std::fs::rename(path, aside);
}

/// 先写到临时文件并落到磁盘，再替换原文件（替换本身也写透到磁盘）：断电时要么是旧的、要么是新的，
/// 不会是空文件（变更日志里存着修改前的原值，丢了就再也还原不了）。临时文件名带上进程号：卸载程序和
/// 正在停止的服务不会写到同一个临时文件里。一时失败（被独占）时隔一会儿再试。
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    retrying(|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace(&tmp, path)
    })
}

fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let (from, to) = (
        WideString::new(&from.to_string_lossy()),
        WideString::new(&to.to_string_lossy()),
    );
    // SAFETY: 两个路径都以 NUL 结尾，调用期间有效。
    unsafe {
        MoveFileExW(
            from.as_pcwstr(),
            to.as_pcwstr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(std::io::Error::other)
}
