//! 变更日志的文件（数据目录的 journal.json）。里面是修改前的原值，丢了就再也还原不了：读不到（文件在却
//! 打不开）时报错而不是当成空的，写入见 [`super::atomic`]。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use poise_core::journal::{JournalEntry, JournalStorage};

use super::atomic::{read_retrying, set_aside, write_atomic};

/// 变更日志存放在数据目录的 journal.json。逐条解析：有读不懂的条目（降级后遇到新版本记的）时只跳过
/// 那几条，原样留着、保存时写回去，不因为一条就丢掉全部原值。
pub struct FileJournal {
    path: PathBuf,
    unknown: RefCell<Vec<serde_json::Value>>,
}

impl FileJournal {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("journal.json"),
            unknown: RefCell::default(),
        }
    }
}

impl JournalStorage for FileJournal {
    fn load(&self) -> Result<Vec<JournalEntry>, String> {
        let path = &self.path;
        let Some(bytes) = read_retrying(path).map_err(|e| e.to_string())? else {
            return Ok(Vec::new());
        };
        let items = match serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
            Ok(items) => items,
            Err(e) => {
                set_aside(path, &e.to_string());
                return Ok(Vec::new());
            }
        };
        let mut entries = Vec::new();
        let mut unknown = Vec::new();
        for item in items {
            match serde_json::from_value::<JournalEntry>(item.clone()) {
                Ok(entry) => entries.push(entry),
                Err(_) => unknown.push(item),
            }
        }
        if !unknown.is_empty() {
            log::warn!(
                "变更日志里有 {} 条读不懂（可能是新版本记的），原样保留",
                unknown.len()
            );
        }
        *self.unknown.borrow_mut() = unknown;
        Ok(entries)
    }

    fn save(&self, entries: &[JournalEntry]) -> Result<(), String> {
        let mut all = serde_json::to_value(entries).map_err(|e| e.to_string())?;
        if let Some(list) = all.as_array_mut() {
            list.extend(self.unknown.borrow().iter().cloned());
        }
        let bytes = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_entries_it_cannot_read_are_kept_and_written_back() {
        let dir = std::env::temp_dir().join(format!("poise-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let journal = FileJournal::new(&dir);
        assert_eq!(journal.load(), Ok(Vec::new()));
        let known = JournalEntry {
            owner: "a".into(),
            target: poise_core::setting::Target::Registry {
                hive: poise_core::setting::Hive::LocalMachine,
                key: "K".into(),
                name: "N".into(),
            },
            original: None,
            applied: poise_core::setting::Value::Dword(1),
            at: 0,
        };
        let mut text = serde_json::to_value(vec![known.clone()]).unwrap();
        text.as_array_mut()
            .unwrap()
            .push(serde_json::json!({"owner": "future", "target": {"Somewhere": {}}}));
        std::fs::write(dir.join("journal.json"), text.to_string()).unwrap();
        assert_eq!(journal.load(), Ok(vec![known.clone()]));
        journal.save(std::slice::from_ref(&known)).unwrap();
        let saved: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(dir.join("journal.json")).unwrap()).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[1]["owner"], "future");
        let _ = std::fs::remove_dir_all(dir);
    }
}
