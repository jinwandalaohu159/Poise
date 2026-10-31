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
    /// 读回日志。文件不存在时是空的；存在但读不到（被别的程序独占、没有权限）时返回错误——
    /// 不能当成空的，否则下一次保存就把里面的原值覆盖掉了。
    fn load(&self) -> Result<Vec<JournalEntry>, String>;
    /// 落盘；失败时返回原因（调用方据此不去改设置：没记下原值的修改不能做）。
    fn save(&self, entries: &[JournalEntry]) -> Result<(), String>;
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

    /// 位置此刻归这里管：不按用户的位置，或者就是当前用户的。另一个用户（快速切换用户、另一个账户）的修改
    /// 是那个用户的事：这里不显示成已改过，也不替他还原。
    pub fn here(store: &dyn SettingStore, target: &Target) -> bool {
        target.user().is_none()
            || store
                .resolve(&target.clone().any_user())
                .is_ok_and(|t| t.same_place(target))
    }

    /// 这个来源在此刻归这里管的位置上有修改（见 [`Journal::here`]）。
    pub fn owns_here(&self, store: &dyn SettingStore, owner: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.owner == owner && Self::here(store, &e.target))
    }

    /// 还原某个来源在此刻归这里管的位置上的修改（见 [`Journal::here`]），别的用户的留着。
    pub fn revert_here(
        &mut self,
        store: &dyn SettingStore,
        owner: &str,
    ) -> Result<(), JournalError> {
        if !self.owns_here(store, owner) {
            return Err(JournalError::NotFound);
        }
        self.revert_where(store, owner, |t| Self::here(store, t))
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

    /// 按日志，这个来源在这个位置上写入的就是 `value`，并且处在最上层：再写一遍没有意义。
    /// 只看日志、不读设置：读设置要经过系统服务，忙的时候（开机、刚切换电源计划）每次几十到几百毫秒。
    /// 被别处改掉的由守护（见 [`Journal::drifted`]）另外发现、改回。
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
            .is_some_and(|i| self.entries[i].applied == *value && self.layer_above(i).is_none())
    }

    /// 这条修改此刻仍在最上层、写入的值没变。别的线程按日志的副本检查（见 [`Journal::drifted`]），
    /// 检查期间日志可能已经变了（例如切换了模式）：结果只对仍然有效的条目适用。
    pub fn is_current(&self, entry: &JournalEntry) -> bool {
        self.entries
            .iter()
            .position(|e| e.owner == entry.owner && e.target.same_place(&entry.target))
            .is_some_and(|i| {
                self.entries[i].applied == entry.applied && self.layer_above(i).is_none()
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

    /// 把一个位置上的条目改回 `before` 里的样子（这次修改没能写下去时：日志要和设置一致）。
    pub fn restore_place(&mut self, before: &Journal, place: &Target) {
        self.entries.retain(|e| !e.target.same_place(place));
        self.entries.extend(
            before
                .entries
                .iter()
                .filter(|e| e.target.same_place(place))
                .cloned(),
        );
    }

    /// 旧版本记下的位置可能是相对的（"当前电源计划"、没写明的当前用户），换成具体的位置。返回是否有变化。
    /// 当前用户还不知道时（服务开机就启动、还没人登录）按用户的位置留着不写明，知道后再调用一次。
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
    use crate::fast_hash::HashMap;
    use std::cell::RefCell;

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
        Target::registry(Hive::CurrentUser, r"Software\Test", name)
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
        let upper = Target::registry(Hive::CurrentUser, r"Software\Test", r"D:\Games\Game.exe");
        let lower = Target::registry(Hive::CurrentUser, r"software\test", r"d:\games\game.exe");
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
        // 被别处改掉了：只看日志时仍算写过（不读设置），由守护发现、改回。
        store.write(&reg("b"), &Value::Dword(9)).unwrap();
        assert!(j.holds(&store, "mode", &reg("b"), &Value::Dword(2)));
        let drifted = j.drifted(&store, |_| true);
        assert_eq!(drifted.len(), 1);
        assert_eq!(drifted[0].1, Some(Value::Dword(9)));
    }

    #[test]
    fn checks_made_on_an_old_copy_only_apply_to_entries_still_in_effect() {
        let store = MemStore::default();
        let mut j = Journal::default();
        j.apply(&store, "mode", &reg("a"), &Value::Dword(1), 0)
            .unwrap();
        let copy = j.clone();
        let entry = copy.entries()[0].clone();
        assert!(j.is_current(&entry));
        // 检查期间换了模式：写入的值变了。
        j.apply(&store, "mode", &reg("a"), &Value::Dword(2), 1)
            .unwrap();
        assert!(!j.is_current(&entry));
        // 别的来源叠在上面：也不再适用。
        let mut j = copy.clone();
        j.apply(&store, "item", &reg("a"), &Value::Dword(3), 2)
            .unwrap();
        assert!(!j.is_current(&entry));
    }

    #[test]
    fn reverting_unknown_owner_is_an_error() {
        let store = MemStore::default();
        let mut j = Journal::default();
        assert_eq!(j.revert(&store, "nope"), Err(JournalError::NotFound));
    }

    /// 当前用户可以换的存储：按用户的位置解析成当前用户。
    #[derive(Default)]
    struct UserStore {
        mem: MemStore,
        current: RefCell<String>,
    }

    impl SettingStore for UserStore {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            self.mem.read(t)
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.mem.write(t, v)
        }
        fn delete(&self, t: &Target) -> Result<(), StoreError> {
            self.mem.delete(t)
        }
        fn resolve(&self, t: &Target) -> Result<Target, StoreError> {
            Ok(if t.user().is_none() {
                t.clone().for_user(&self.current.borrow())
            } else {
                t.clone()
            })
        }
    }

    #[test]
    fn each_user_keeps_their_own_originals() {
        let store = UserStore::default();
        *store.current.borrow_mut() = "A".into();
        let mut j = Journal::default();
        j.apply(&store, "item", &reg("a"), &Value::Dword(1), 1)
            .unwrap();
        assert!(j.owns_here(&store, "item"));
        // 换成另一个用户：A 的修改不算这里的，也不替他还原；B 自己应用一次，原值各记各的。
        *store.current.borrow_mut() = "B".into();
        assert!(!j.owns_here(&store, "item"));
        assert_eq!(j.revert_here(&store, "item"), Err(JournalError::NotFound));
        j.apply(&store, "item", &reg("a"), &Value::Dword(1), 2)
            .unwrap();
        assert_eq!(j.entries().len(), 2);
        j.revert_here(&store, "item").unwrap();
        assert_eq!(j.entries().len(), 1);
        assert_eq!(j.entries()[0].target.user(), Some("A"));
    }
}
