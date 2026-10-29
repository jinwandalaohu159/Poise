//! 通用层：各模式对应的电源计划参数。全部经由变更日志修改（叠在系统优化项之上），Poise 退出时撤销。
//!
//! 什么时候升频、升多少，由处理器按实际负载自己决定（硬件自主调频 + 能效偏好），Poise 不替它判断。
//! Poise 只保证两件事：处理器的动态范围是完整的，以及各模式的能效偏好。
//!
//! - 所有模式的底线：最小处理器状态取 Windows 平衡计划的出厂值（空闲时能降下来），最大处理器状态 100%
//!   （有负载时能升上去），核心停放也取出厂值（插电时小核不停放：后台程序和让路的程序都在小核上，
//!   实测小核被停放到只剩几个时，它们挤在一起排队），每个效率等级都设。用户的计划被别的程序改坏了（实测有电脑开机后被改成
//!   最小状态 100%、睿频被关，处理器空闲时不降频、有负载时又只能跑到基础频率）也不受影响。
//! - 均衡：睿频方式与能效偏好也取 Windows 平衡计划的出厂值。
//! - 全力与机型极限：大核的性能全部放开；大小核处理器上其余核心保持均衡的能效偏好——
//!   后台程序在小核上不必跑满频率，省下的功耗和温度留给游戏所在的大核（笔记本的处理器和显卡共用散热，
//!   所有核心都跑满频率只会更早撞上温度墙）。
//! - 安静：关闭睿频，能效偏好偏向节能。
//!
//! [`baseline`]：Windows 平衡计划在这台电脑上的出厂值；[`modes`]：各模式要写入的值；这里负责写入与撤销。

mod baseline;
mod modes;

pub use baseline::Baseline;
pub use modes::settings_for;

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::power_settings::{BOOST_MODE, CP_MIN_CORES, EPP, MAX_STATE, MIN_STATE, target};
use poise_core::prefs::Mode;
use poise_core::setting::{SettingStore, Value};

/// 变更日志里属于电源模式的条目。
pub const OWNER: &str = "hardware:power";

/// 写入某个模式的电源设置：先撤销上一个模式写入的值，再写入新的，保证不同模式的设置不会残留叠加。
/// 整个过程是一批写入；返回写入结果，以及是否需要重新激活电源计划才能生效——
/// 激活要两三百毫秒，由调用方放到硬件执行线程里做（见 `actuator`）。
pub fn write(
    mode: Mode,
    top_class: u8,
    baseline: &Baseline,
    store: &dyn SettingStore,
    journal: &mut Journal,
    now: Millis,
) -> (Result<(), JournalError>, bool) {
    batch(store, || {
        revert_owned(store, journal)?;
        for (t, v) in settings_for(mode, top_class, baseline) {
            journal.apply(store, OWNER, &t, &v, now)?;
        }
        Ok(())
    })
}

/// 撤销模式写入的全部电源设置（Poise 退出、服务停止时）。返回值同 [`write`]。
pub fn revert(store: &dyn SettingStore, journal: &mut Journal) -> (Result<(), JournalError>, bool) {
    batch(store, || revert_owned(store, journal))
}

fn revert_owned(store: &dyn SettingStore, journal: &mut Journal) -> Result<(), JournalError> {
    match journal.revert(store, OWNER) {
        Ok(()) | Err(JournalError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

fn batch(
    store: &dyn SettingStore,
    f: impl FnOnce() -> Result<(), JournalError>,
) -> (Result<(), JournalError>, bool) {
    store.begin_batch();
    let result = f();
    let activate = store.end_batch_deferred();
    (result, activate)
}

/// 处理器此刻实际的电源设置（日志用，确认设置真的生效了）。
pub fn describe(store: &dyn SettingStore, top_class: u8, ac: bool) -> String {
    let top = usize::from(top_class).min(EPP.len() - 1);
    let read = |s: &str| match store.read(&target(s, ac)) {
        Ok(Some(Value::Dword(v))) => v.to_string(),
        _ => "?".into(),
    };
    let per_class = |list: &[&str]| {
        list.iter()
            .take(top + 1)
            .map(|s| read(s))
            .collect::<Vec<_>>()
            .join("/")
    };
    format!(
        "最小状态 {}%，最大状态 {}%，睿频方式 {}，能效偏好 {}，核心停放最少保留 {}%",
        per_class(&MIN_STATE),
        per_class(&MAX_STATE),
        read(BOOST_MODE),
        per_class(&EPP),
        per_class(&CP_MIN_CORES)
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use poise_core::setting::{StoreError, Target};

    use super::*;

    const HYBRID: u8 = 1;

    fn apply(mode: Mode, store: &dyn SettingStore, journal: &mut Journal) {
        write(mode, HYBRID, &Baseline::default(), store, journal, 0)
            .0
            .unwrap();
    }

    /// 没写过的设置读到 50。
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

    fn read(store: &Mem, setting: &str) -> Option<Value> {
        store.read(&target(setting, true)).unwrap()
    }

    #[test]
    fn reverting_restores_original_values() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, &store, &mut j);
        assert_eq!(read(&store, CP_MIN_CORES[1]), Some(Value::Dword(100)));
        revert(&store, &mut j).0.unwrap();
        assert_eq!(read(&store, CP_MIN_CORES[1]), Some(Value::Dword(50)));
        assert_eq!(read(&store, CP_MIN_CORES[0]), Some(Value::Dword(50)));
        assert_eq!(read(&store, MIN_STATE[1]), Some(Value::Dword(50)));
        assert!(j.entries().is_empty());
    }

    #[test]
    fn switching_between_modes_does_not_leak_settings() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, &store, &mut j);
        apply(Mode::Quiet, &store, &mut j);
        // 每个设置只有一条（最新模式的）记录，撤销后回到最初的原值。
        let owned = j.entries().iter().filter(|e| e.owner == OWNER).count();
        assert_eq!(
            owned,
            settings_for(Mode::Quiet, HYBRID, &Baseline::default()).len()
        );
        revert(&store, &mut j).0.unwrap();
        assert_eq!(read(&store, BOOST_MODE), Some(Value::Dword(50)));
    }

    #[test]
    fn modes_stack_on_top_of_optimizations_and_leave_them_in_place() {
        let store = Mem::default();
        let mut j = Journal::default();
        j.apply(
            &store,
            "cpu_idle_downclock",
            &target(MIN_STATE[1], true),
            &Value::Dword(5),
            0,
        )
        .unwrap();
        apply(Mode::Full, &store, &mut j);
        revert(&store, &mut j).0.unwrap();
        assert!(j.owns("cpu_idle_downclock"));
        assert_eq!(read(&store, MIN_STATE[1]), Some(Value::Dword(5)));
    }

    #[test]
    fn describes_the_settings_per_class() {
        let store = Mem::default();
        let mut j = Journal::default();
        apply(Mode::Full, &store, &mut j);
        assert_eq!(
            describe(&store, HYBRID, true),
            "最小状态 5/5%，最大状态 100/100%，睿频方式 2，能效偏好 50/0，核心停放最少保留 100/100%"
        );
    }
}
