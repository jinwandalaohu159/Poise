//! JSON 文件持久化：先写临时文件再替换，避免写到一半断电导致文件损坏。

use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use poise_core::journal::{JournalEntry, JournalStorage};
use serde::Serialize;
use serde::de::DeserializeOwned;

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

    pub fn save(&self, value: &T) -> std::io::Result<()> {
        let tmp = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &self.path)
    }
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
        if let Err(e) = self.0.save(&entries.to_vec()) {
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
