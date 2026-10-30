//! 处理器电源设置（电源计划"处理器电源管理"子组）的 GUID，优化项与硬件模式共用。
//!
//! 很多设置按处理器效率等级分成几个版本：基础版本作用于等级 0（最省电的核心，大小核处理器上是小核），
//! "第 N 类"版本作用于等级 N，大核的等级最高。不是大小核的处理器只用到等级 0，其余版本照样存在、写入无妨。
//! 只改基础版本的话，大小核处理器的大核完全不受影响——所以凡是分等级的设置，都按等级列全。

use serde::{Deserialize, Serialize};

use crate::setting::Target;

pub const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";

/// 最小处理器状态（百分比），按等级 0、1、2。
pub const MIN_STATE: [&str; 3] = [
    "893dee8e-2bef-41e0-89c6-b55d0929964c",
    "893dee8e-2bef-41e0-89c6-b55d0929964d",
    "893dee8e-2bef-41e0-89c6-b55d0929964e",
];

/// Windows"平衡"计划的最小处理器状态。
pub const IDLE_MIN_STATE: u32 = 5;
/// 最小处理器状态不超过这个值，空闲时就能降下来（超过就是在强制高频）。
pub const IDLE_MIN_STATE_LIMIT: u32 = 50;

/// 最大处理器状态（百分比），按等级 0、1、2。
pub const MAX_STATE: [&str; 3] = [
    "bc5038f7-23e0-4960-96da-33abaf5935ec",
    "bc5038f7-23e0-4960-96da-33abaf5935ed",
    "bc5038f7-23e0-4960-96da-33abaf5935ee",
];

/// 能效偏好（EPP）：0 最偏性能，100 最偏节能。按等级 0、1、2。
pub const EPP: [&str; 3] = [
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6863",
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6864",
    "36687f9e-e3a5-4dbf-b1dc-15eb381c6865",
];

/// 核心停放：最少保持多少百分比的核心不休眠。按等级 0、1。
pub const CP_MIN_CORES: [&str; 2] = [
    "0cc5b647-c1df-4637-891a-dec35c318583",
    "0cc5b647-c1df-4637-891a-dec35c318584",
];

/// 睿频模式：0 禁用（处理器最高只跑到基础频率），1 启用，2 激进（Windows 默认），其余为各种"高效"变体。
pub const BOOST_MODE: &str = "be337238-0d82-4146-a960-4f3749d470c7";
pub const BOOST_DISABLED: u32 = 0;
pub const BOOST_AGGRESSIVE: u32 = 2;

/// 当前电源计划中处理器子组的某个设置。
pub fn target(setting: &str, ac: bool) -> Target {
    Target::power(SUB_PROCESSOR, setting, ac)
}

/// 处理器电源设置的种类（日志与界面按它显示名称）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSettingKind {
    MinState,
    MaxState,
    Epp,
    CoreParkingMin,
    BoostMode,
}

/// 一个设置是哪种、作用于哪个效率等级（不分等级的为 None）；不认识的为 None。
pub fn kind(setting: &str) -> Option<(PowerSettingKind, Option<u8>)> {
    let classed = |list: &[&str], kind: PowerSettingKind| {
        list.iter()
            .position(|s| s.eq_ignore_ascii_case(setting))
            .map(|class| (kind, Some(class as u8)))
    };
    classed(&MIN_STATE, PowerSettingKind::MinState)
        .or_else(|| classed(&MAX_STATE, PowerSettingKind::MaxState))
        .or_else(|| classed(&EPP, PowerSettingKind::Epp))
        .or_else(|| classed(&CP_MIN_CORES, PowerSettingKind::CoreParkingMin))
        .or_else(|| {
            setting
                .eq_ignore_ascii_case(BOOST_MODE)
                .then_some((PowerSettingKind::BoostMode, None))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_variants_differ_only_in_the_last_digit() {
        for list in [&MIN_STATE[..], &MAX_STATE[..], &EPP[..], &CP_MIN_CORES[..]] {
            for pair in list.windows(2) {
                assert_eq!(pair[0][..35], pair[1][..35]);
                assert_ne!(pair[0], pair[1]);
            }
        }
    }

    #[test]
    fn kinds_say_which_class() {
        assert_eq!(
            kind(&MIN_STATE[1].to_uppercase()),
            Some((PowerSettingKind::MinState, Some(1)))
        );
        assert_eq!(kind(BOOST_MODE), Some((PowerSettingKind::BoostMode, None)));
        assert_eq!(kind("unknown"), None);
    }
}
