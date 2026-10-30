//! 变更日志：所有持久修改都先记下原值，确保每一项都能单独还原。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::Millis;
use crate::setting::{SettingStore, StoreError, Target, Value};

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

    /// 修改一个设置。
    ///
    /// 同一个位置可以有几层修改（例如系统优化项打底，硬件模式临时叠在上面），按修改的先后排列：
    /// 上层记下的原值就是下层写入的值，撤销上层回到下层的值。同一来源再次修改同一位置时保留最初的原值；
    /// 这时它上面还有别的层的话不写入（生效的仍是上层的值），只更新上层撤销后要回到的值。
    pub fn apply(
        &mut self,
        store: &dyn SettingStore,
        owner: &str,
        target: &Target,
        value: &Value,
        now: Millis,
    ) -> Result<(), JournalError> {
        let target = store.resolve(target).map_err(JournalError::Read)?;
        let existing = self
            .entries
            .iter()
            .position(|e| e.owner == owner && e.target.same_place(&target));
        match existing {
            Some(i) => {
                match self.layer_above(i) {
                    Some(above) => self.entries[above].original = Some(value.clone()),
                    None => store.write(&target, value).map_err(JournalError::Write)?,
                }
                let e = &mut self.entries[i];
                e.applied = value.clone();
                e.at = now;
            }
            None => {
                let original = store.read(&target).map_err(JournalError::Read)?;
                store.write(&target, value).map_err(JournalError::Write)?;
                self.entries.push(JournalEntry {
                    owner: owner.to_string(),
                    target,
                    original,
                    applied: value.clone(),
                    at: now,
                });
            }
        }
        Ok(())
    }

    /// 同一位置上紧挨着的上一层。
    fn layer_above(&self, i: usize) -> Option<usize> {
        let place = &self.entries[i].target;
        (i + 1..self.entries.len()).find(|&k| self.entries[k].target.same_place(place))
    }

    /// 还原某个来源的所有修改。上面还有别的层的位置不写入：把原值交给上一层，等它撤销时再回到原值。
    pub fn revert(&mut self, store: &dyn SettingStore, owner: &str) -> Result<(), JournalError> {
        if !self.owns(owner) {
            return Err(JournalError::NotFound);
        }
        self.revert_where(store, owner, |_| true)
    }

    /// 还原某个来源在部分位置上的修改（`drop` 选中的），其余保留。规则同 [`Journal::revert`]。
    pub fn revert_where(
        &mut self,
        store: &dyn SettingStore,
        owner: &str,
        drop: impl Fn(&Target) -> bool,
    ) -> Result<(), JournalError> {
        let mut first_error = None;
        for i in (0..self.entries.len()).rev() {
            let e = &self.entries[i];
            if e.owner != owner || !drop(&e.target) {
                continue;
            }
            match self.layer_above(i) {
                Some(above) => self.entries[above].original = self.entries[i].original.clone(),
                None => {
                    if let Err(err) = restore(store, &self.entries[i]) {
                        first_error.get_or_insert(err);
                        continue;
                    }
                }
            }
            self.entries.remove(i);
        }
        first_error.map_or(Ok(()), |e| Err(JournalError::Revert(e)))
    }

    /// 这个来源在这个位置上已经写入了 `value`，并且此刻生效的正是它（处在最上层、没被别处改掉）：
    /// 再写一遍没有意义。
    pub fn holds(
        &self,
        store: &dyn SettingStore,
        owner: &str,
        target: &Target,
        value: &Value,
    ) -> bool {
        let Ok(target) = store.resolve(target) else {
            return false;
        };
        self.entries
            .iter()
            .position(|e| e.owner == owner && e.target.same_place(&target))
            .is_some_and(|i| {
                self.entries[i].applied == *value
                    && self.layer_above(i).is_none()
                    && store.read(&target).is_ok_and(|v| v.as_ref() == Some(value))
            })
    }

    /// 还原全部修改，返回失败的来源。按修改的相反顺序进行（同一位置的几层逐层撤销）。
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

    /// 被别处改掉的修改：`watch` 选中的记录里，处在最上层、当前值却已不是它写入的值的，连同当前值一起返回。
    /// 读不到的跳过。
    pub fn drifted(
        &self,
        store: &dyn SettingStore,
        watch: impl Fn(&JournalEntry) -> bool,
    ) -> Vec<(JournalEntry, Option<Value>)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, e)| watch(e) && self.layer_above(*i).is_none())
            .filter_map(|(_, e)| match store.read(&e.target) {
                Ok(current) if current.as_ref() != Some(&e.applied) => Some((e.clone(), current)),
                _ => None,
            })
            .collect()
    }

    /// `owner` 在这个位置的修改被别的来源叠在上面（例如优化项打底、当前模式叠在上面）：
    /// 此刻生效的是上层的值，它自己的值等上层撤销后回来。没有这条修改、读不到位置时为否。
    pub fn covered(&self, store: &dyn SettingStore, owner: &str, target: &Target) -> bool {
        let Ok(target) = store.resolve(target) else {
            return false;
        };
        self.entries
            .iter()
            .position(|e| e.owner == owner && e.target.same_place(&target))
            .is_some_and(|i| self.layer_above(i).is_some())
    }

    /// 旧版本记下的位置可能是相对的（例如"当前电源计划"），换成具体的位置。返回是否有变化。
    pub fn resolve_places(&mut self, store: &dyn SettingStore) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if let Ok(t) = store.resolve(&e.target)
                && t != e.target
            {
                e.target = t;
                changed = true;
            }
        }
        changed
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
    use crate::setting::Hive;

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
    fn layers_on_one_setting_unwind_in_any_order() {
        let store = MemStore::default();
        store.write(&reg("boost"), &Value::Dword(0)).unwrap();
        let mut j = Journal::default();
        // 优化项打底，模式叠在上面。
        j.apply(&store, "opt", &reg("boost"), &Value::Dword(2), 1)
            .unwrap();
        j.apply(&store, "mode", &reg("boost"), &Value::Dword(3), 2)
            .unwrap();
        // 先撤销模式：回到优化项的值。
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&reg("boost")).unwrap(), Some(Value::Dword(2)));
        assert!(j.owns("opt"));

        // 再叠一次模式，这次先撤销下层的优化项：生效的仍是模式的值，模式撤销时回到最初的原值。
        j.apply(&store, "mode", &reg("boost"), &Value::Dword(3), 3)
            .unwrap();
        j.revert(&store, "opt").unwrap();
        assert_eq!(store.read(&reg("boost")).unwrap(), Some(Value::Dword(3)));
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&reg("boost")).unwrap(), Some(Value::Dword(0)));
        assert!(j.entries().is_empty());
    }

    #[test]
    fn a_lower_layer_reapplying_does_not_override_the_layer_above() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "opt", &reg("v"), &Value::Dword(5), 1)
            .unwrap();
        j.apply(&store, "mode", &reg("v"), &Value::Dword(9), 2)
            .unwrap();
        j.apply(&store, "opt", &reg("v"), &Value::Dword(6), 3)
            .unwrap();
        assert_eq!(store.read(&reg("v")).unwrap(), Some(Value::Dword(9)));
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&reg("v")).unwrap(), Some(Value::Dword(6)));
    }

    #[test]
    fn places_that_differ_only_in_case_are_one_place() {
        let store = MemStore::default();
        let mut j = Journal::default();
        let upper = Target::Registry {
            hive: Hive::CurrentUser,
            key: r"Software\Test".into(),
            name: r"D:\Games\Game.exe".into(),
        };
        let lower = Target::Registry {
            hive: Hive::CurrentUser,
            key: r"software\test".into(),
            name: r"d:\games\game.exe".into(),
        };
        j.apply(&store, "g", &upper, &Value::Dword(1), 1).unwrap();
        j.apply(&store, "g", &lower, &Value::Dword(2), 2).unwrap();
        assert_eq!(j.entries().len(), 1);
    }

    #[test]
    fn only_the_top_layer_changed_elsewhere_counts_as_drifted() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "opt", &reg("a"), &Value::Dword(5), 1)
            .unwrap();
        j.apply(&store, "mode", &reg("a"), &Value::Dword(9), 2)
            .unwrap();
        j.apply(&store, "opt", &reg("b"), &Value::Dword(5), 3)
            .unwrap();
        assert!(j.drifted(&store, |_| true).is_empty());
        store.write(&reg("a"), &Value::Dword(100)).unwrap();
        store.write(&reg("b"), &Value::Dword(100)).unwrap();
        let drifted = j.drifted(&store, |e| e.owner == "mode" || e.owner == "opt");
        let owners: Vec<_> = drifted.iter().map(|(e, _)| e.owner.as_str()).collect();
        assert_eq!(owners, ["mode", "opt"]);
        assert_eq!(drifted[0].1, Some(Value::Dword(100)));
    }

    #[test]
    fn a_layer_under_another_owner_is_covered() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "item", &reg("a"), &Value::Dword(1), 0)
            .unwrap();
        assert!(!j.covered(&store, "item", &reg("a")));
        j.apply(&store, "mode", &reg("a"), &Value::Dword(0), 1)
            .unwrap();
        assert!(j.covered(&store, "item", &reg("a")));
        assert!(!j.covered(&store, "mode", &reg("a")));
        assert!(!j.covered(&store, "other", &reg("a")));
        j.revert(&store, "mode").unwrap();
        assert!(!j.covered(&store, "item", &reg("a")));
    }

    #[test]
    fn partial_reverts_keep_the_rest_and_held_values_are_recognized() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "mode", &reg("a"), &Value::Dword(1), 0)
            .unwrap();
        j.apply(&store, "mode", &reg("b"), &Value::Dword(2), 0)
            .unwrap();
        assert!(j.holds(&store, "mode", &reg("a"), &Value::Dword(1)));
        assert!(!j.holds(&store, "mode", &reg("a"), &Value::Dword(3)));
        j.revert_where(&store, "mode", |t| t.same_place(&reg("a")))
            .unwrap();
        assert_eq!(store.read(&reg("a")).unwrap(), None);
        assert!(j.holds(&store, "mode", &reg("b"), &Value::Dword(2)));
        // 被别处改掉了：不算。
        store.write(&reg("b"), &Value::Dword(9)).unwrap();
        assert!(!j.holds(&store, "mode", &reg("b"), &Value::Dword(2)));
    }

    #[test]
    fn reverting_unknown_owner_is_an_error() {
        let store = MemStore::default();
        let mut j = Journal::default();
        assert_eq!(j.revert(&store, "nope"), Err(JournalError::NotFound));
    }
}
