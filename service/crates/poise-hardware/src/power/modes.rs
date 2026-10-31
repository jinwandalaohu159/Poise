//! 各模式要写入的电源设置（见上级模块的说明）。纯函数，不读写任何东西。

use poise_core::power_settings::{
    BOOST_AGGRESSIVE, BOOST_DISABLED, BOOST_MODE, CP_MIN_CORES, EPP, MAX_STATE, MIN_STATE, target,
};
use poise_core::prefs::Mode;
use poise_core::setting::{Target, Value};

use super::Baseline;

/// 全力模式下大核以外的核心的能效偏好：均衡（Windows 均衡计划的默认值附近）。
const EFFICIENT_EPP: u32 = 50;
/// 安静模式的能效偏好：大核、其余核心。
const QUIET_EPP: (u32, u32) = (70, 85);
const FULL_STATE: u32 = 100;

/// 一个设置在插电、用电池时的取值。
type Pair = (&'static str, u32, u32);

fn same(setting: &'static str, v: u32) -> Pair {
    (setting, v, v)
}

fn from_baseline(b: &Baseline, setting: &'static str) -> Pair {
    (setting, b.get(setting, true), b.get(setting, false))
}

/// 处理器的组成（决定电源设置按哪些效率等级写）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuShape {
    /// 大核的效率等级（见 `CpuTopology::top_class`），不是大小核时为 0。
    pub top_class: u8,
    /// 大核的几组核心缓存大小不同（7950X3D 这样一组带 3D 缓存）：全力模式也按出厂值停放核心——
    /// 处理器驱动靠停放把游戏留在缓存大的那组上（反作弊保护的游戏 Poise 不能限制核心范围，只能靠它）。
    pub uneven_cache: bool,
}

/// 某个模式需要写入的设置。
pub fn settings_for(mode: Mode, cpu: CpuShape, baseline: &Baseline) -> Vec<(Target, Value)> {
    let top = usize::from(cpu.top_class).min(EPP.len() - 1);
    // 核心停放只分两级：等级 1 的设置管等级 1 及以上的核心（有三个等级的处理器，大核在等级 2）。
    let top_parking = top.min(CP_MIN_CORES.len() - 1);
    let classes = || EPP.iter().enumerate().take(top + 1);
    let full = matches!(mode, Mode::Full | Mode::Vendor);
    let mut values: Vec<Pair> = Vec::new();
    // 底线：动态范围完整。全力模式下大核不停放（缓存大小不同的除外，见 `CpuShape`）。
    // 大小核处理器插电时小核不停放：让路的程序、给重点程序让出大核的程序都在小核上，
    // 小核被停放到只剩几个时它们挤在一起排队（实测有机型的出厂值只保留 4%）。
    values.extend(MIN_STATE.iter().map(|s| from_baseline(baseline, s)));
    values.extend(MAX_STATE.iter().map(|s| same(s, FULL_STATE)));
    values.extend(CP_MIN_CORES.iter().enumerate().map(|(class, s)| {
        if full && class == top_parking && !cpu.uneven_cache {
            same(s, FULL_STATE)
        } else if top > 0 && class == 0 {
            (*s, FULL_STATE, baseline.get(s, false))
        } else {
            from_baseline(baseline, s)
        }
    }));
    match mode {
        Mode::Balanced | Mode::Auto => {
            values.push(from_baseline(baseline, BOOST_MODE));
            values.extend(classes().map(|(_, s)| from_baseline(baseline, s)));
        }
        Mode::Full | Mode::Vendor => {
            values.push(same(BOOST_MODE, BOOST_AGGRESSIVE));
            values.extend(
                classes().map(|(class, s)| same(s, if class == top { 0 } else { EFFICIENT_EPP })),
            );
        }
        Mode::Quiet => {
            values.push(same(BOOST_MODE, BOOST_DISABLED));
            values.extend(classes().map(|(class, s)| {
                let (top_epp, other_epp) = QUIET_EPP;
                same(s, if class == top { top_epp } else { other_epp })
            }));
        }
    }
    values
        .into_iter()
        .flat_map(|(s, ac, dc)| {
            [
                (target(s, true), Value::Dword(ac)),
                (target(s, false), Value::Dword(dc)),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HYBRID: u8 = 1;

    /// 某个模式给某个设置的取值；没写这个设置时为 None。
    fn value(
        mode: Mode,
        top_class: u8,
        baseline: &Baseline,
        setting: &str,
        ac: bool,
    ) -> Option<u32> {
        let cpu = CpuShape {
            top_class,
            uneven_cache: false,
        };
        settings_for(mode, cpu, baseline)
            .into_iter()
            .find(|(t, _)| t == &target(setting, ac))
            .map(|(_, v)| match v {
                Value::Dword(n) => n,
                other => panic!("{other:?}"),
            })
    }

    #[test]
    fn full_mode_frees_only_the_performance_cores_on_hybrid_cpus() {
        let b = Baseline::default();
        let full = |s, ac| value(Mode::Full, HYBRID, &b, s, ac);
        assert_eq!(full(EPP[1], true), Some(0));
        assert_eq!(full(EPP[0], true), Some(EFFICIENT_EPP));
        assert_eq!(full(CP_MIN_CORES[1], true), Some(100));
        // 小核的核心停放按出厂值：插电时不停放。
        assert_eq!(full(CP_MIN_CORES[0], true), Some(100));
        assert_eq!(full(CP_MIN_CORES[0], false), Some(10));
    }

    #[test]
    fn small_cores_are_never_parked_on_ac_even_if_the_factory_value_parks_them() {
        let dell = Baseline::from_lookup(|s, _| (s == CP_MIN_CORES[0]).then_some(4));
        for mode in [Mode::Quiet, Mode::Balanced, Mode::Full] {
            assert_eq!(value(mode, HYBRID, &dell, CP_MIN_CORES[0], true), Some(100));
            assert_eq!(value(mode, HYBRID, &dell, CP_MIN_CORES[0], false), Some(4));
        }
        // 不是大小核：按出厂值。
        assert_eq!(
            value(Mode::Balanced, 0, &dell, CP_MIN_CORES[0], true),
            Some(4)
        );
    }

    #[test]
    fn full_mode_frees_every_core_on_uniform_cpus() {
        let b = Baseline::default();
        assert_eq!(value(Mode::Full, 0, &b, EPP[0], true), Some(0));
        assert_eq!(value(Mode::Full, 0, &b, CP_MIN_CORES[0], true), Some(100));
        // 不存在的效率等级不写能效偏好。
        assert_eq!(value(Mode::Full, 0, &b, EPP[1], true), None);
    }

    #[test]
    fn three_class_cpus_unpark_the_performance_cores_in_full_mode() {
        let b = Baseline::default();
        assert_eq!(value(Mode::Full, 2, &b, CP_MIN_CORES[1], true), Some(100));
        assert_eq!(value(Mode::Full, 2, &b, EPP[2], true), Some(0));
    }

    #[test]
    fn cpus_with_uneven_caches_keep_parking_cores_in_full_mode() {
        let b = Baseline::from_lookup(|s, _| (s == CP_MIN_CORES[0]).then_some(10));
        let x3d = CpuShape {
            top_class: 0,
            uneven_cache: true,
        };
        let parking = settings_for(Mode::Full, x3d, &b)
            .into_iter()
            .find(|(t, _)| t == &target(CP_MIN_CORES[0], true))
            .map(|(_, v)| v);
        assert_eq!(parking, Some(Value::Dword(10)));
        assert_eq!(value(Mode::Full, 0, &b, CP_MIN_CORES[0], true), Some(100));
    }

    #[test]
    fn every_mode_keeps_the_full_dynamic_range_on_every_core_class() {
        let b = Baseline::default();
        for mode in [Mode::Quiet, Mode::Balanced, Mode::Full, Mode::Vendor] {
            for ac in [true, false] {
                for s in MIN_STATE {
                    assert_eq!(value(mode, HYBRID, &b, s, ac), Some(5), "{mode:?}");
                }
                for s in MAX_STATE {
                    assert_eq!(value(mode, HYBRID, &b, s, ac), Some(100), "{mode:?}");
                }
            }
        }
    }

    #[test]
    fn balanced_follows_the_factory_values() {
        let oem = Baseline::from_lookup(|s, ac| (s == EPP[1] && ac).then_some(25));
        let balanced = |s, ac| value(Mode::Balanced, HYBRID, &oem, s, ac);
        assert_eq!(balanced(BOOST_MODE, true), Some(BOOST_AGGRESSIVE));
        assert_eq!(balanced(EPP[1], true), Some(25));
        assert_eq!(balanced(EPP[0], true), Some(33));
        assert_eq!(balanced(EPP[0], false), Some(50));
        // 安静模式下大核按出厂值可以停放。
        assert_eq!(
            value(Mode::Quiet, HYBRID, &oem, CP_MIN_CORES[1], true),
            Some(0)
        );
        assert_eq!(value(Mode::Quiet, HYBRID, &oem, BOOST_MODE, false), Some(0));
    }
}
