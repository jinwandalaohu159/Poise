//! 系统优化：只收录"改一次、长期有效、可以还原"的设置。

mod catalog;
mod model;

pub use catalog::static_items;
pub use model::{
    Group, Hive, ItemState, Matcher, OptimizationItem, SettingStore, StoreError, SystemDefault,
    TakesEffect, Target, Text, UserFile, Value,
};

use serde::{Deserialize, Serialize};

use crate::journal::{Journal, JournalError};
use crate::model::Millis;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStatus {
    pub item: OptimizationItem,
    pub state: ItemState,
    /// 可以还原：Poise 改过（还原到修改前），或者已是优化状态且知道系统默认值（还原到默认）。
    pub revertible: bool,
}

pub fn status(
    items: &[OptimizationItem],
    store: &dyn SettingStore,
    journal: &Journal,
) -> Vec<ItemStatus> {
    items
        .iter()
        .map(|item| {
            let state = match store.read(&item.target) {
                Ok(v) if item.matcher.is_optimized(v.as_ref(), &item.optimized) => {
                    ItemState::Optimized
                }
                Ok(_) => ItemState::NotOptimized,
                Err(_) => ItemState::Unknown,
            };
            ItemStatus {
                revertible: journal.owns(&item.id)
                    || (state == ItemState::Optimized && item.system_default.is_known()),
                state,
                item: item.clone(),
            }
        })
        .collect()
}

/// 把不是 Poise 改的设置恢复为系统默认。已经不是优化状态时什么都不做（不覆盖用户自己的其他取值）；
/// 不知道默认值时返回 [`StoreError::Unsupported`]。
pub fn restore_default(
    item: &OptimizationItem,
    store: &dyn SettingStore,
) -> Result<(), StoreError> {
    if !item.system_default.is_known() {
        return Err(StoreError::Unsupported);
    }
    let current = store.read(&item.target)?;
    if !item.matcher.is_optimized(current.as_ref(), &item.optimized) {
        return Ok(());
    }
    match &item.system_default {
        SystemDefault::Unknown => Err(StoreError::Unsupported),
        SystemDefault::Absent => store.delete(&item.target),
        SystemDefault::Value(v) => store.write(&item.target, v),
    }
}

pub fn apply(
    item: &OptimizationItem,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> Result<(), JournalError> {
    journal.apply(store, &item.id, &item.target, &item.optimized, now)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// 一键优化：只处理标记为一键优化、且尚未优化的项。
pub fn apply_recommended(
    items: &[OptimizationItem],
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> ApplyReport {
    let mut report = ApplyReport::default();
    for s in status(items, store, journal) {
        if !s.item.in_one_click || s.state != ItemState::NotOptimized {
            continue;
        }
        match apply(&s.item, store, journal, now) {
            Ok(()) => report.applied.push(s.item.id),
            Err(e) => report.failed.push((s.item.id, e.to_string())),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::tests::{MemStore, reg};

    fn item(id: &str, one_click: bool) -> OptimizationItem {
        OptimizationItem {
            id: id.into(),
            group: if one_click {
                Group::Explorer
            } else {
                Group::ContextMenu
            },
            title: Text::Plain(id.into()),
            description: Text::Plain(String::new()),
            target: reg(id),
            optimized: Value::Dword(1),
            matcher: Matcher::Exact,
            in_one_click: one_click,
            slow_hint: false,
            takes_effect: TakesEffect::Immediately,
            system_default: SystemDefault::Unknown,
        }
    }

    #[test]
    fn one_click_skips_per_item_groups_and_already_optimized() {
        let store = MemStore::default();
        store.write(&reg("done"), &Value::Dword(1)).unwrap();
        let items = [item("a", true), item("done", true), item("plugin", false)];
        let mut j = Journal::default();
        let r = apply_recommended(&items, &store, &mut j, 0);
        assert_eq!(r.applied, vec!["a".to_string()]);
        assert_eq!(store.read(&reg("plugin")).unwrap(), None);
    }

    #[test]
    fn status_reflects_store_and_journal() {
        let store = MemStore::default();
        let items = [item("a", true)];
        let mut j = Journal::default();
        assert_eq!(status(&items, &store, &j)[0].state, ItemState::NotOptimized);
        apply(&items[0], &store, &mut j, 0).unwrap();
        let s = &status(&items, &store, &j)[0];
        assert_eq!(s.state, ItemState::Optimized);
        assert!(s.revertible);
    }

    #[test]
    fn settings_changed_elsewhere_can_go_back_to_the_system_default() {
        let store = MemStore::default();
        store.write(&reg("a"), &Value::Dword(1)).unwrap();
        let unknown = item("a", true);
        let j = Journal::default();
        assert!(!status(std::slice::from_ref(&unknown), &store, &j)[0].revertible);
        assert_eq!(
            restore_default(&unknown, &store),
            Err(StoreError::Unsupported)
        );

        let absent = OptimizationItem {
            system_default: SystemDefault::Absent,
            ..unknown
        };
        let s = &status(std::slice::from_ref(&absent), &store, &j)[0];
        assert_eq!(s.state, ItemState::Optimized);
        assert!(s.revertible);
        restore_default(&absent, &store).unwrap();
        assert_eq!(store.read(&reg("a")).unwrap(), None);
        assert!(!status(&[absent], &store, &j)[0].revertible);
    }

    #[test]
    fn restoring_the_default_keeps_other_user_values() {
        let store = MemStore::default();
        store.write(&reg("a"), &Value::Dword(7)).unwrap();
        let it = OptimizationItem {
            system_default: SystemDefault::Absent,
            ..item("a", true)
        };
        restore_default(&it, &store).unwrap();
        assert_eq!(store.read(&reg("a")).unwrap(), Some(Value::Dword(7)));
    }
}
