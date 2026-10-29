//! 通用层：各模式对应的电源计划参数。全部经由变更日志修改，回到"均衡"时还原。
//! 全力与机型极限模式把大核的性能全部放开；大小核处理器上，其余核心保持均衡的能效偏好——
//! 后台程序在小核上不必跑满频率，省下的功耗和温度留给游戏所在的大核（笔记本的处理器和显卡共用散热，
//! 所有核心都跑满频率只会更早撞上温度墙）。不改用户原来的电源计划，切回其他模式时还原。
//!
//! 按效率等级区分的设置：Windows 的效率等级从 0（最省电的核心）开始编号，大核的等级最高。
//! 设置的基础版本作用于等级 0，"第 N 类"版本作用于等级 N；不是大小核的处理器只有等级 0。

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::{SettingStore, Target, Value};
use poise_core::prefs::Mode;

/// 变更日志里属于电源模式的条目。
pub const OWNER: &str = "hardware:power";

const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
/// 睿频模式：0 禁用，2 激进。
const PERF_BOOST_MODE: &str = "be337238-0d82-4146-a960-4f3749d470c7";
/// 能效偏好（EPP）：0 最偏性能，100 最偏节能。按效率等级 0、1、2。
const PERF_EPP: [&str; 3] = [
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6863",
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6864",
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6865",
];
/// 最大处理器状态（百分比）。
const PROC_MAX: &str = "bc5038f7-23e0-4960-96da-33abaf5935ec";
/// 核心停放：最少保持多少百分比的核心不休眠。按效率等级 0、1。
const CP_MIN_CORES: [&str; 2] = [
    "0cc5b647-c1df-4637-891a-dec35c318583",
    "0cc5b647-c1df-4637-891a-dec35c318584",
];

/// 全力模式下大核以外的核心的能效偏好：均衡（Windows 均衡计划的默认值附近）。
const EFFICIENT_EPP: u32 = 50;

fn target(setting: &str, ac: bool) -> Target {
    Target::PowerSetting {
        subgroup: SUB_PROCESSOR.into(),
        setting: setting.into(),
        ac,
    }
}

/// 某个模式需要写入的设置；均衡模式返回空（表示还原到原始值）。
/// `top_class` 是大核的效率等级（见 `CpuTopology::top_class`），不是大小核时为 0。
pub fn settings_for(mode: Mode, top_class: u8) -> Vec<(Target, Value)> {
    let top = usize::from(top_class).min(PERF_EPP.len() - 1);
    let mut values: Vec<(&str, u32)> = Vec::new();
    match mode {
        Mode::Balanced | Mode::Auto => {}
        Mode::Full | Mode::Vendor => {
            values.extend([(PROC_MAX, 100), (PERF_BOOST_MODE, 2)]);
            for (class, epp) in PERF_EPP.iter().enumerate().take(top + 1) {
                values.push((epp, if class == top { 0 } else { EFFICIENT_EPP }));
            }
            if let Some(cp) = CP_MIN_CORES.get(top) {
                values.push((cp, 100));
            }
        }
        Mode::Quiet => {
            values.push((PERF_BOOST_MODE, 0));
            for (class, epp) in PERF_EPP.iter().enumerate().take(top + 1) {
                values.push((epp, if class == top { 70 } else { 85 }));
            }
        }
    }
    values
        .iter()
        .flat_map(|(s, v)| [true, false].map(|ac| (target(s, ac), Value::Dword(*v))))
        .collect()
}

/// 写入某个模式的电源设置：先还原上一个模式写入的值，再写入新的，保证不同模式的设置不会残留叠加。
/// 整个过程是一批写入；返回写入结果，以及是否需要重新激活电源计划才能生效——
/// 激活要两三百毫秒，由调用方放到硬件执行线程里做（见 `actuator`）。
pub fn write(
    mode: Mode,
    top_class: u8,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> (Result<(), JournalError>, bool) {
    store.begin_batch();
    let result = apply_values(mode, top_class, store, journal, now);
    let activate = store.end_batch_deferred();
    (result, activate)
}

fn apply_values(
    mode: Mode,
    top_class: u8,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> Result<(), JournalError> {
    match journal.revert(store, OWNER) {
        Ok(()) | Err(JournalError::NotFound) => {}
        Err(e) => return Err(e),
    }
    for (t, v) in settings_for(mode, top_class) {
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

    fn apply(
        mode: Mode,
        top_class: u8,
        store: &dyn SettingStore,
        journal: &mut Journal,
        now: Millis,
    ) -> Result<(), JournalError> {
        write(mode, top_class, store, journal, now).0
    }

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

    const HYBRID: u8 = 1;

    fn read(store: &Mem, setting: &str) -> Option<Value> {
        store.read(&target(setting, true)).unwrap()
    }

    #[test]
    fn full_mode_frees_only_the_performance_cores_on_hybrid_cpus() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, HYBRID, &store, &mut j, 0).unwrap();
        assert_eq!(read(&store, PERF_EPP[1]), Some(Value::Dword(0)));
        assert_eq!(read(&store, PERF_EPP[0]), Some(Value::Dword(EFFICIENT_EPP)));
        assert_eq!(read(&store, CP_MIN_CORES[1]), Some(Value::Dword(100)));
        // 小核的核心停放不改。
        assert_eq!(read(&store, CP_MIN_CORES[0]), Some(Value::Dword(50)));
    }

    #[test]
    fn full_mode_frees_every_core_on_uniform_cpus() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, 0, &store, &mut j, 0).unwrap();
        assert_eq!(read(&store, PERF_EPP[0]), Some(Value::Dword(0)));
        assert_eq!(read(&store, CP_MIN_CORES[0]), Some(Value::Dword(100)));
        assert_eq!(read(&store, PERF_EPP[1]), Some(Value::Dword(50)));
    }

    #[test]
    fn full_then_balanced_restores_original_values() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, HYBRID, &store, &mut j, 0).unwrap();
        assert_eq!(read(&store, CP_MIN_CORES[1]), Some(Value::Dword(100)));
        apply(Mode::Balanced, HYBRID, &store, &mut j, 1).unwrap();
        assert_eq!(
            store.read(&target(CP_MIN_CORES[1], true)).unwrap(),
            Some(Value::Dword(50))
        );
        assert!(j.entries().is_empty());
    }

    #[test]
    fn switching_between_modes_does_not_leak_settings() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, HYBRID, &store, &mut j, 0).unwrap();
        apply(Mode::Quiet, HYBRID, &store, &mut j, 1).unwrap();
        // 安静模式不涉及核心停放，它应已回到原值。
        assert_eq!(
            store.read(&target(CP_MIN_CORES[1], true)).unwrap(),
            Some(Value::Dword(50))
        );
        assert_eq!(
            store.read(&target(PERF_BOOST_MODE, false)).unwrap(),
            Some(Value::Dword(0))
        );
    }
}
