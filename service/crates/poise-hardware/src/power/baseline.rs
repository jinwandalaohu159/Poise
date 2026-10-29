//! Windows"平衡"计划在这台电脑上的出厂值：均衡模式与所有模式的底线按它设置。
//!
//! 出厂值按计划的"个性"读取（`PowerReadACDefaultIndex`），与任何程序对计划的修改无关。但出厂值也可能
//! 被厂商改坏，所以每个值都要满足平衡计划该有的样子——空闲时能降下来、有负载时能升上去、能效偏好不在
//! 两端——不满足的、读不到的，都用 Windows 的通用默认值。

use std::collections::HashMap;
use std::ops::RangeInclusive;

use poise_core::power_settings::{
    BOOST_AGGRESSIVE, BOOST_DISABLED, BOOST_MODE, CP_MIN_CORES, EPP, IDLE_MIN_STATE,
    IDLE_MIN_STATE_LIMIT, MIN_STATE, SUB_PROCESSOR,
};
use poise_platform::settings::power_plan;

/// 能效偏好落在两端，就是"始终满频"或"始终省电"，不是均衡。
const BALANCED_EPP: RangeInclusive<u32> = 10..=90;
/// 睿频方式的取值范围（0 禁用，1～6 为各种启用方式）。
const BOOST_MODES: RangeInclusive<u32> = 0..=6;

#[derive(Debug, Clone, Default)]
pub struct Baseline {
    values: HashMap<(&'static str, bool), u32>,
}

impl Baseline {
    /// 读取这台电脑的出厂值。
    pub fn read() -> Self {
        let sub = power_plan::parse_guid(SUB_PROCESSOR);
        Self::from_lookup(|setting, ac| {
            let setting = power_plan::parse_guid(setting)?;
            power_plan::default_value(&power_plan::BALANCED, sub.as_ref()?, &setting, ac).ok()
        })
    }

    /// 按 `lookup` 给出的出厂值；没有的、不合理的用通用默认值。
    pub fn from_lookup(lookup: impl Fn(&str, bool) -> Option<u32>) -> Self {
        let settings = MIN_STATE
            .iter()
            .chain(&EPP)
            .chain(&CP_MIN_CORES)
            .chain(&[BOOST_MODE]);
        let mut values = HashMap::new();
        for setting in settings {
            for ac in [true, false] {
                let v = lookup(setting, ac)
                    .filter(|&v| sensible(setting, v))
                    .unwrap_or_else(|| generic_default(setting, ac));
                values.insert((*setting, ac), v);
            }
        }
        Self { values }
    }

    pub(super) fn get(&self, setting: &'static str, ac: bool) -> u32 {
        self.values
            .get(&(setting, ac))
            .copied()
            .unwrap_or_else(|| generic_default(setting, ac))
    }
}

/// 出厂值是否像个平衡计划：空闲能降、负载能升、能效偏好不在两端。
fn sensible(setting: &str, v: u32) -> bool {
    if MIN_STATE.contains(&setting) {
        v <= IDLE_MIN_STATE_LIMIT
    } else if setting == BOOST_MODE {
        v != BOOST_DISABLED && BOOST_MODES.contains(&v)
    } else if EPP.contains(&setting) {
        BALANCED_EPP.contains(&v)
    } else {
        v <= 100
    }
}

/// Windows 平衡计划的通用默认值。
fn generic_default(setting: &str, ac: bool) -> u32 {
    if MIN_STATE.contains(&setting) {
        IDLE_MIN_STATE
    } else if EPP.contains(&setting) {
        if ac { 33 } else { 50 }
    } else if setting == BOOST_MODE {
        BOOST_AGGRESSIVE
    } else if setting == CP_MIN_CORES[0] {
        // 最省电的核心：插电时不停放。
        if ac { 100 } else { 10 }
    } else if CP_MIN_CORES.contains(&setting) {
        // 其余核心：空闲时可以全部停放，有负载时再唤醒。
        0
    } else {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oem_values_are_used_when_they_look_like_a_balanced_plan() {
        let oem = Baseline::from_lookup(|s, ac| (s == EPP[1] && ac).then_some(45));
        assert_eq!(oem.get(EPP[1], true), 45);
        // 没有的用通用默认值。
        assert_eq!(oem.get(EPP[0], true), 33);
        assert_eq!(oem.get(EPP[0], false), 50);
        assert_eq!(oem.get(BOOST_MODE, true), BOOST_AGGRESSIVE);
    }

    #[test]
    fn broken_factory_values_fall_back_to_the_windows_defaults() {
        // 厂商把"平衡"的出厂值也改成了高性能的样子：最小状态 100%、睿频禁用、能效偏好 0。
        let broken = Baseline::from_lookup(|s, _| {
            if MIN_STATE.contains(&s) {
                Some(100)
            } else if s == BOOST_MODE {
                Some(BOOST_DISABLED)
            } else if EPP.contains(&s) {
                Some(0)
            } else {
                Some(250)
            }
        });
        assert_eq!(broken.get(MIN_STATE[1], true), IDLE_MIN_STATE);
        assert_eq!(broken.get(BOOST_MODE, true), BOOST_AGGRESSIVE);
        assert_eq!(broken.get(EPP[1], true), 33);
        assert_eq!(broken.get(CP_MIN_CORES[0], true), 100);
    }

    #[test]
    fn reads_this_machine_without_failing() {
        let b = Baseline::read();
        assert!(b.get(MIN_STATE[0], true) <= IDLE_MIN_STATE_LIMIT);
        assert_ne!(b.get(BOOST_MODE, true), BOOST_DISABLED);
    }
}
