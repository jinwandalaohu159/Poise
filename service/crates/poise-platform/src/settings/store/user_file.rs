//! 用户目录下的 INI 配置文件（例如 `.wslconfig`）。只改指定的键，文件里的其他内容原样保留。
//!
//! 服务以 SYSTEM 身份写用户目录：用户目录归用户自己管，文件被换成指向用户目录以外的链接时不读不写，
//! 否则就成了替用户以 SYSTEM 身份改别处的文件。指向用户目录里的（配置文件管理工具常这样做）照常读写；
//! 用户目录本身是链接（挪到了别的盘）也照常——它的位置来自只有管理员能改的注册表。

use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use poise_core::setting::{StoreError, UserFile, Value};

use crate::persist::atomic::write_atomic;
use crate::settings::ini;
use crate::settings::user::UserContext;

fn path_of(user: Option<&UserContext>, file: UserFile) -> Result<PathBuf, StoreError> {
    let user = user.ok_or(StoreError::Unsupported)?;
    let path = match file {
        UserFile::WslConfig => user.profile_dir.join(".wslconfig"),
    };
    if is_reparse_point(&path) && !points_inside(&path, &user.profile_dir) {
        return Err(StoreError::AccessDenied);
    }
    Ok(path)
}

/// 链接最终指向 `dir` 里面（两边都解析到最终的真实路径再比）。
fn points_inside(link: &Path, dir: &Path) -> bool {
    match (std::fs::canonicalize(link), std::fs::canonicalize(dir)) {
        (Ok(target), Ok(dir)) => target.starts_with(dir),
        _ => false,
    }
}

fn is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

fn io_err(e: std::io::Error) -> StoreError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        StoreError::AccessDenied
    } else {
        StoreError::Io(e.to_string())
    }
}

/// 文件不存在时当作空文件。
fn read_text(path: &PathBuf) -> Result<String, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_err(e)),
    }
}

pub(super) fn read(
    user: Option<&UserContext>,
    file: UserFile,
    section: &str,
    key: &str,
) -> Result<Option<Value>, StoreError> {
    let text = read_text(&path_of(user, file)?)?;
    Ok(ini::get(&text, section, key).map(Value::String))
}

pub(super) fn write(
    user: Option<&UserContext>,
    file: UserFile,
    section: &str,
    key: &str,
    value: &Value,
) -> Result<(), StoreError> {
    let Value::String(v) = value else {
        return Err(StoreError::Unsupported);
    };
    let path = path_of(user, file)?;
    let text = read_text(&path)?;
    write_atomic(&path, ini::set(&text, section, key, v).as_bytes()).map_err(io_err)
}

pub(super) fn delete(
    user: Option<&UserContext>,
    file: UserFile,
    section: &str,
    key: &str,
) -> Result<(), StoreError> {
    let path = path_of(user, file)?;
    let text = read_text(&path)?;
    if text.is_empty() {
        return Ok(());
    }
    write_atomic(&path, ini::remove(&text, section, key).as_bytes()).map_err(io_err)
}
