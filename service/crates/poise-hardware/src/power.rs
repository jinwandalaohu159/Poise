//! 通用层：三档模式对应的电源计划参数。全部经由变更日志修改，回到"均衡"时还原。

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::{SettingStore, Target, Value};
use poise_core::prefs::Mode;

/// 变更日志里属于电源模式的条目。
pub const OWNER: &str = "hardware:power";

const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
/// 睿频模式：0 禁用，2 激进。
const PERF_BOOST_MODE: &str = "be337238-0d82-4146-a960-4f3749d470c7";
/// 能效偏好（EPP）：0 最偏性能，100 最偏节能。P 核 / E 核分别设置。
const PERF_EPP: &str = "36687f9e-e3a5-4dbf-b1dc-15eb381c6863";
const PERF_EPP_E: &str = "36687f9e-e3a5-4dbf-b1dc-15eb381c6864";
/// 核心停放：最少保持多少百分比的核心不休眠。
const CP_MIN_CORES: &str = "0cc5b647-c1df-4637-891a-dec35c318583";
const CP_MIN_CORES_E: &str = "0cc5b647-c1df-4637-891a-dec35c318584";

fn target(setting: &str, ac: bool) -> Target {
    Target::PowerSetting {
        subgroup: SUB_PROCESSOR.into(),
        setting: setting.into(),
        ac,
    }
}

/// 某个模式需要写入的设置；均衡模式返回空（表示还原到原始值）。
pub fn settings_for(mode: Mode) -> Vec<(Target, Value)> {
    let values: &[(&str, u32)] = match mode {
        Mode::Balanced => &[],
        Mode::Full => &[
            (PERF_BOOST_MODE, 2),
            (PERF_EPP, 0),
            (PERF_EPP_E, 0),
            (CP_MIN_CORES, 100),
            (CP_MIN_CORES_E, 100),
        ],
        Mode::Quiet => &[(PERF_BOOST_MODE, 0), (PERF_EPP, 70), (PERF_EPP_E, 85)],
    };
    values
        .iter()
        .flat_map(|(s, v)| [true, false].map(|ac| (target(s, ac), Value::Dword(*v))))
        .collect()
}

/// 应用模式。切换模式前先还原，保证不同模式的设置不会残留叠加。
pub fn apply(
    mode: Mode,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> Result<(), JournalError> {
    match journal.revert(store, OWNER) {
        Ok(()) | Err(JournalError::NotFound) => {}
        Err(e) => return Err(e),
    }
    for (t, v) in settings_for(mode) {
        journal.apply(store, OWNER, &t, &v, now)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use poise_core::optimize::StoreError;

    use super::*;

    #[derive(Default)]
    struct Mem(RefCell<HashMap<Target, Value>>);
    impl SettingStore for Mem {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            Ok(Some(
                self.0.borrow().get(t).cloned().unwrap_or(Value::Dword(50)),
            ))
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.0.borrow_mut().insert(t.clone(), v.clone());
            Ok(())
        }
        fn delete(&self, _: &Target) -> Result<(), StoreError> {
            Err(StoreError::Unsupported)
        }
    }

    #[test]
    fn full_then_balanced_restores_original_values() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, &store, &mut j, 0).unwrap();
        assert_eq!(
            store.read(&target(CP_MIN_CORES, true)).unwrap(),
            Some(Value::Dword(100))
        );
        apply(Mode::Balanced, &store, &mut j, 1).unwrap();
        assert_eq!(
            store.read(&target(CP_MIN_CORES, true)).unwrap(),
            Some(Value::Dword(50))
        );
        assert!(j.entries().is_empty());
    }

    #[test]
    fn switching_between_modes_does_not_leak_settings() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, &store, &mut j, 0).unwrap();
        apply(Mode::Quiet, &store, &mut j, 1).unwrap();
        // 安静模式不涉及核心停放，它应已回到原值。
        assert_eq!(
            store.read(&target(CP_MIN_CORES, true)).unwrap(),
            Some(Value::Dword(50))
        );
        assert_eq!(
            store.read(&target(PERF_BOOST_MODE, false)).unwrap(),
            Some(Value::Dword(0))
        );
    }
}
