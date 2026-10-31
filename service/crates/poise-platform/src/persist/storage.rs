//! JSON 文件持久化：先写临时文件再替换，避免写到一半断电导致文件损坏。

use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::atomic::{read_retrying, set_aside, write_atomic};
use super::writer::FileWriter;

pub struct JsonFile<T> {
    path: PathBuf,
    _t: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> JsonFile<T> {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            _t: PhantomData,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 文件不存在、读不到或损坏时返回 None（损坏的文件会被改名保留，便于排查）。
    pub fn load(&self) -> Option<T> {
        let bytes = match read_retrying(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(e) => {
                log::warn!("读不到 {}：{e}", self.path.display());
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                set_aside(&self.path, &e.to_string());
                None
            }
        }
    }

    /// 立即保存（见 [`write_atomic`]）：变更日志这样必须先落盘、再去修改设置的用它。
    pub fn save(&self, value: &T) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
        write_atomic(&self.path, &bytes)
    }

    /// 交给后台线程保存（见 [`FileWriter`]），不等磁盘。
    pub fn save_later(&self, value: &T, writer: &FileWriter) {
        match serde_json::to_vec_pretty(value) {
            Ok(bytes) => writer.submit(self.path.clone(), bytes),
            Err(e) => log::warn!("保存 {} 失败：{e}", self.path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_and_corrupt_is_set_aside() {
        let dir = std::env::temp_dir().join(format!("poise-storage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f: JsonFile<Vec<u32>> = JsonFile::new(dir.join("x.json"));
        assert!(f.load().is_none());
        f.save(&vec![1, 2, 3]).unwrap();
        assert_eq!(f.load(), Some(vec![1, 2, 3]));
        std::fs::write(f.path(), b"{broken").unwrap();
        assert!(f.load().is_none());
        let set_aside = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".corrupt"));
        assert!(set_aside);
        let _ = std::fs::remove_dir_all(dir);
    }
}
