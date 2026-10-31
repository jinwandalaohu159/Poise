//! JSON 文件持久化：先写临时文件再替换，避免写到一半断电导致文件损坏。

use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use poise_core::journal::{JournalEntry, JournalStorage};
use serde::Serialize;
use serde::de::DeserializeOwned;

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

    /// 文件不存在或损坏时返回 None（损坏的文件会被改名保留，便于排查）。
    pub fn load(&self) -> Option<T> {
        let bytes = std::fs::read(&self.path).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("{} 无法解析（{e}），已改名保留", self.path.display());
                let _ = std::fs::rename(&self.path, self.path.with_extension("corrupt"));
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

/// 先写到临时文件并落到磁盘，再替换原文件：断电时要么是旧的、要么是新的，不会是空文件
/// （变更日志里存着修改前的原值，丢了就再也还原不了）。
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

/// 变更日志存放在数据目录的 journal.json。
pub struct FileJournal(JsonFile<Vec<JournalEntry>>);

impl FileJournal {
    pub fn new(dir: &Path) -> Self {
        Self(JsonFile::new(dir.join("journal.json")))
    }
}

impl JournalStorage for FileJournal {
    fn load(&self) -> Vec<JournalEntry> {
        self.0.load().unwrap_or_default()
    }

    fn save(&self, entries: &[JournalEntry]) {
        let saved = serde_json::to_vec_pretty(entries)
            .map_err(std::io::Error::other)
            .and_then(|bytes| write_atomic(self.0.path(), &bytes));
        if let Err(e) = saved {
            log::error!("保存变更日志失败：{e}");
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
        assert!(dir.join("x.corrupt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
