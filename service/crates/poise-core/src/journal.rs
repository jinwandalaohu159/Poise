//! 变更日志：所有持久修改都先记下原值，确保每一项都能单独还原。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::Millis;
use crate::optimize::{SettingStore, StoreError, Target, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// 产生这条修改的来源（优化项 ID、硬件模式等）。
    pub owner: String,
    pub target: Target,
    /// 修改前的值；None 表示原本不存在，还原时删除。
    pub original: Option<Value>,
    pub applied: Value,
    pub at: Millis,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum JournalError {
    #[error("读取原值失败：{0}")]
    Read(StoreError),
    #[error("写入失败：{0}")]
    Write(StoreError),
    #[error("还原失败：{0}")]
    Revert(StoreError),
    #[error("没有这条修改记录")]
    NotFound,
}

/// 平台层实现的日志持久化。
pub trait JournalStorage {
    fn load(&self) -> Vec<JournalEntry>;
    fn save(&self, entries: &[JournalEntry]);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Journal {
    entries: Vec<JournalEntry>,
}

impl Journal {
    pub fn from_entries(entries: Vec<JournalEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    pub fn owns(&self, owner: &str) -> bool {
        self.entries.iter().any(|e| e.owner == owner)
    }

    /// 修改一个设置。同一位置已有记录时保留最初的原值，只更新新值。
    pub fn apply(
        &mut self,
        store: &dyn SettingStore,
        owner: &str,
        target: &Target,
        value: &Value,
        now: Millis,
    ) -> Result<(), JournalError> {
        match self.entries.iter_mut().find(|e| &e.target == target) {
            Some(existing) => {
                store.write(target, value).map_err(JournalError::Write)?;
                existing.applied = value.clone();
                existing.owner = owner.to_string();
                existing.at = now;
            }
            None => {
                let original = store.read(target).map_err(JournalError::Read)?;
                store.write(target, value).map_err(JournalError::Write)?;
                self.entries.push(JournalEntry {
                    owner: owner.to_string(),
                    target: target.clone(),
                    original,
                    applied: value.clone(),
                    at: now,
                });
            }
        }
        Ok(())
    }

    /// 还原某个来源的所有修改。
    pub fn revert(&mut self, store: &dyn SettingStore, owner: &str) -> Result<(), JournalError> {
        if !self.owns(owner) {
            return Err(JournalError::NotFound);
        }
        let mut first_error = None;
        self.entries.retain(|e| {
            if e.owner != owner {
                return true;
            }
            match restore(store, e) {
                Ok(()) => false,
                Err(err) => {
                    first_error.get_or_insert(err);
                    true
                }
            }
        });
        first_error.map_or(Ok(()), |e| Err(JournalError::Revert(e)))
    }

    /// 还原全部修改，返回失败的来源。按修改的相反顺序进行。
    pub fn revert_all(&mut self, store: &dyn SettingStore) -> Vec<(String, JournalError)> {
        let mut failures = Vec::new();
        let mut kept = Vec::new();
        for e in self.entries.drain(..).rev() {
            if let Err(err) = restore(store, &e) {
                failures.push((e.owner.clone(), JournalError::Revert(err)));
                kept.push(e);
            }
        }
        kept.reverse();
        self.entries = kept;
        failures
    }
}

fn restore(store: &dyn SettingStore, e: &JournalEntry) -> Result<(), StoreError> {
    match &e.original {
        Some(v) => store.write(&e.target, v),
        None => store.delete(&e.target),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::optimize::Hive;

    /// 内存里的设置存储，供测试使用。
    #[derive(Default)]
    pub struct MemStore {
        pub values: RefCell<HashMap<Target, Value>>,
    }

    impl SettingStore for MemStore {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            Ok(self.values.borrow().get(t).cloned())
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.values.borrow_mut().insert(t.clone(), v.clone());
            Ok(())
        }
        fn delete(&self, t: &Target) -> Result<(), StoreError> {
            self.values.borrow_mut().remove(t);
            Ok(())
        }
    }

    pub fn reg(name: &str) -> Target {
        Target::Registry {
            hive: Hive::CurrentUser,
            key: r"Software\Test".into(),
            name: name.into(),
        }
    }

    #[test]
    fn revert_restores_original_value() {
        let store = MemStore::default();
        store.write(&reg("a"), &Value::Dword(1)).unwrap();
        let mut j = Journal::default();
        j.apply(&store, "item", &reg("a"), &Value::Dword(0), 1)
            .unwrap();
        assert_eq!(store.read(&reg("a")).unwrap(), Some(Value::Dword(0)));
        j.revert(&store, "item").unwrap();
        assert_eq!(store.read(&reg("a")).unwrap(), Some(Value::Dword(1)));
        assert!(j.entries().is_empty());
    }

    #[test]
    fn revert_deletes_values_that_did_not_exist() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "item", &reg("b"), &Value::String(String::new()), 1)
            .unwrap();
        j.revert(&store, "item").unwrap();
        assert_eq!(store.read(&reg("b")).unwrap(), None);
    }

    #[test]
    fn repeated_apply_keeps_the_first_original() {
        let store = MemStore::default();
        store.write(&reg("c"), &Value::Dword(4)).unwrap();
        let mut j = Journal::default();
        j.apply(&store, "mode", &reg("c"), &Value::Dword(100), 1)
            .unwrap();
        j.apply(&store, "mode", &reg("c"), &Value::Dword(50), 2)
            .unwrap();
        assert_eq!(j.entries().len(), 1);
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&reg("c")).unwrap(), Some(Value::Dword(4)));
    }

    #[test]
    fn revert_all_returns_everything_to_original() {
        let store = MemStore::default();
        store.write(&reg("x"), &Value::Dword(1)).unwrap();
        let mut j = Journal::default();
        j.apply(&store, "a", &reg("x"), &Value::Dword(2), 1)
            .unwrap();
        j.apply(&store, "b", &reg("y"), &Value::Dword(3), 2)
            .unwrap();
        assert!(j.revert_all(&store).is_empty());
        assert_eq!(store.read(&reg("x")).unwrap(), Some(Value::Dword(1)));
        assert_eq!(store.read(&reg("y")).unwrap(), None);
    }

    #[test]
    fn reverting_unknown_owner_is_an_error() {
        let store = MemStore::default();
        let mut j = Journal::default();
        assert_eq!(j.revert(&store, "nope"), Err(JournalError::NotFound));
    }
}
