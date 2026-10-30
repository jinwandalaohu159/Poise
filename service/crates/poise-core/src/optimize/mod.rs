//! 系统优化：只收录"改一次、长期有效、可以还原"的设置。

mod catalog;
mod model;

pub use catalog::{power_items, static_items};
pub use model::{Group, ItemState, Matcher, OptimizationItem, SystemDefault, TakesEffect, Text};

use serde::{Deserialize, Serialize};

use crate::journal::{Journal, JournalError};
use crate::model::Millis;
use crate::setting::{SettingStore, StoreError, Target};

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
            let state = state(item, store, journal);
            ItemStatus {
                revertible: journal.owns(&item.id)
                    || (state == ItemState::Optimized && item.system_default.is_known()),
                state,
                item: item.clone(),
            }
        })
        .collect()
}

/// 每一处都是优化值才算已优化；有一处不是就算未优化。被别的来源暂时盖住的位置（例如安静模式关掉了睿频）
/// 算已优化：下面仍是优化值，上层撤销后就回来，不必（也不能）重新应用。
fn state(item: &OptimizationItem, store: &dyn SettingStore, journal: &Journal) -> ItemState {
    let mut state = ItemState::Optimized;
    for target in &item.targets {
        if journal.covered(store, &item.id, target) {
            continue;
        }
        match store.read(target) {
            Ok(v) if item.is_optimized(v.as_ref()) => {}
            Ok(_) => return ItemState::NotOptimized,
            Err(_) => state = ItemState::Unknown,
        }
    }
    state
}

/// 把不是 Poise 改的设置恢复为系统默认。已经不是优化状态的位置什么都不做（不覆盖用户自己的其他取值）；
/// 不知道默认值时返回 [`StoreError::Unsupported`]。
pub fn restore_default(
    item: &OptimizationItem,
    store: &dyn SettingStore,
) -> Result<(), StoreError> {
    if !item.system_default.is_known() {
        return Err(StoreError::Unsupported);
    }
    store.begin_batch();
    let result = item
        .targets
        .iter()
        .try_for_each(|target| restore_default_at(item, target, store));
    store.end_batch();
    result
}

fn restore_default_at(
    item: &OptimizationItem,
    target: &Target,
    store: &dyn SettingStore,
) -> Result<(), StoreError> {
    let current = store.read(target)?;
    if !item.is_optimized(current.as_ref()) {
        return Ok(());
    }
    match &item.system_default {
        SystemDefault::Unknown => Err(StoreError::Unsupported),
        SystemDefault::Absent => store.delete(target),
        SystemDefault::Value(v) => store.write(target, v),
    }
}

/// 应用一项：每一处都写入优化值（一批写入，电源计划只重新激活一次）。
pub fn apply(
    item: &OptimizationItem,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> Result<(), JournalError> {
    store.begin_batch();
    let result = item
        .targets
        .iter()
        .try_for_each(|target| journal.apply(store, &item.id, target, &item.optimized, now));
    store.end_batch();
    result
}

/// Poise 应用过、现在却不是优化状态的电源项：厂商工具、系统更新、切换电源计划都可能把电源设置悄悄改回去，
/// 调用方应重新应用。也包括应用之后新增了位置的项（例如新版本覆盖了更多处理器等级），重新应用会补上。
/// 只看电源设置：其他设置被改回，多半是用户自己在系统里改的，不去覆盖。
pub fn reverted_power_items<'a>(
    items: &'a [OptimizationItem],
    store: &dyn SettingStore,
    journal: &Journal,
) -> Vec<&'a OptimizationItem> {
    items
        .iter()
        .filter(|item| {
            item.group == Group::Power
                && journal.owns(&item.id)
                && state(item, store, journal) == ItemState::NotOptimized
        })
        .collect()
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
    store.begin_batch();
    for s in status(items, store, journal) {
        if !s.item.in_one_click || s.state != ItemState::NotOptimized {
            continue;
        }
        match apply(&s.item, store, journal, now) {
            Ok(()) => report.applied.push(s.item.id),
            Err(e) => report.failed.push((s.item.id, e.to_string())),
        }
    }
    store.end_batch();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::tests::{MemStore, reg};
    use crate::setting::Value;

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
            targets: vec![reg(id)],
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

    fn power(id: &str, targets: &[&str]) -> OptimizationItem {
        OptimizationItem {
            group: Group::Power,
            targets: targets.iter().map(|t| reg(t)).collect(),
            optimized: Value::Dword(5),
            matcher: Matcher::AtMost(50),
            ..item(id, true)
        }
    }

    #[test]
    fn every_target_must_be_optimized_and_all_are_written_and_reverted() {
        let store = MemStore::default();
        for t in ["class0", "class1"] {
            store.write(&reg(t), &Value::Dword(100)).unwrap();
        }
        // 只有一处是优化值：仍算未优化（大核还锁在最高频率）。
        store.write(&reg("class0"), &Value::Dword(5)).unwrap();
        let items = [power("idle", &["class0", "class1"])];
        let mut j = Journal::default();
        assert_eq!(status(&items, &store, &j)[0].state, ItemState::NotOptimized);

        apply(&items[0], &store, &mut j, 0).unwrap();
        assert_eq!(status(&items, &store, &j)[0].state, ItemState::Optimized);
        assert_eq!(store.read(&reg("class1")).unwrap(), Some(Value::Dword(5)));

        j.revert(&store, "idle").unwrap();
        assert_eq!(store.read(&reg("class0")).unwrap(), Some(Value::Dword(5)));
        assert_eq!(store.read(&reg("class1")).unwrap(), Some(Value::Dword(100)));
    }

    #[test]
    fn power_items_changed_back_elsewhere_are_found_but_other_items_are_left_alone() {
        let store = MemStore::default();
        let items = [power("idle", &["class0", "class1"]), item("startup", true)];
        let mut j = Journal::default();
        for i in &items {
            apply(i, &store, &mut j, 0).unwrap();
        }
        assert!(reverted_power_items(&items, &store, &j).is_empty());

        // 厂商工具把大核改回 100%，用户把启动项改了回去。
        store.write(&reg("class1"), &Value::Dword(100)).unwrap();
        store.write(&reg("startup"), &Value::Dword(0)).unwrap();
        let reverted = reverted_power_items(&items, &store, &j);
        assert_eq!(reverted.len(), 1);
        assert_eq!(reverted[0].id, "idle");
        apply(reverted[0], &store, &mut j, 1).unwrap();
        assert_eq!(store.read(&reg("class1")).unwrap(), Some(Value::Dword(5)));

        // 还原仍回到最初的原值（不存在则删除）。
        j.revert(&store, "idle").unwrap();
        assert_eq!(store.read(&reg("class1")).unwrap(), None);
    }

    #[test]
    fn power_items_covered_by_a_mode_count_as_optimized_and_are_not_reapplied() {
        let store = MemStore::default();
        let items = [power("idle", &["class0"])];
        let mut j = Journal::default();
        apply(&items[0], &store, &mut j, 0).unwrap();
        // 当前模式在同一位置叠了别的值（例如安静模式关掉睿频）：不是被别处改掉，不重新应用。
        j.apply(&store, "mode", &reg("class0"), &Value::Dword(100), 1)
            .unwrap();
        assert!(reverted_power_items(&items, &store, &j).is_empty());
        assert_eq!(status(&items, &store, &j)[0].state, ItemState::Optimized);
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&reg("class0")).unwrap(), Some(Value::Dword(5)));
    }

    #[test]
    fn power_items_the_user_never_applied_are_not_forced() {
        let store = MemStore::default();
        store.write(&reg("class0"), &Value::Dword(100)).unwrap();
        let items = [power("idle", &["class0"])];
        assert!(reverted_power_items(&items, &store, &Journal::default()).is_empty());
    }
}
