//! CPU 拓扑：通过 EfficiencyClass 自动识别大核（性能核）与小核（能效核），不写死核心编号。
//! 效率等级最高的是大核，其余全部算小核（兼容有两种以上小核的处理器）。

use std::collections::HashSet;

use poise_core::action::CoreSet;
use windows::Win32::System::SystemInformation::{
    CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
};
use windows::Win32::System::Threading::GetCurrentProcess;

/// 一类核心的数量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoreCount {
    pub cores: u32,
    pub threads: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuTopology {
    /// 大核的 CPU Set ID。不是大小核架构时为空。
    pub performance: Vec<u32>,
    /// 小核的 CPU Set ID。不是大小核架构时为空。
    pub efficiency: Vec<u32>,
    /// 大核数量；不是大小核架构时是全部核心。
    pub performance_count: CoreCount,
    /// 小核数量；不是大小核架构时为 0。
    pub efficiency_count: CoreCount,
    /// 第 0 组各逻辑处理器是否是大核（按逻辑处理器序号）。不是大小核架构时为空。
    pub performance_by_index: Vec<bool>,
}

/// 系统报告的一个 CPU Set。
#[derive(Debug, Clone, Copy)]
struct CpuSetEntry {
    id: u32,
    group: u16,
    logical_index: u8,
    core_index: u8,
    efficiency_class: u8,
}

impl CpuTopology {
    pub fn detect() -> Self {
        Self::from_sets(&read_cpu_sets())
    }

    fn from_sets(sets: &[CpuSetEntry]) -> Self {
        let Some(max) = sets.iter().map(|s| s.efficiency_class).max() else {
            return Self::default();
        };
        let (big, small): (Vec<&CpuSetEntry>, Vec<&CpuSetEntry>) =
            sets.iter().partition(|s| s.efficiency_class == max);
        let count = |group: &[&CpuSetEntry]| CoreCount {
            cores: group
                .iter()
                .map(|s| (s.group, s.core_index))
                .collect::<HashSet<_>>()
                .len() as u32,
            threads: group.len() as u32,
        };
        let mut t = Self {
            performance_count: count(&big),
            efficiency_count: count(&small),
            ..Self::default()
        };
        if small.is_empty() {
            // 不是大小核架构：不限制核心，只用优先级和节能区分前后台。
            return t;
        }
        t.performance = big.iter().map(|s| s.id).collect();
        t.efficiency = small.iter().map(|s| s.id).collect();
        let group0 = sets.iter().filter(|s| s.group == 0);
        let len = group0
            .clone()
            .map(|s| s.logical_index as usize + 1)
            .max()
            .unwrap_or(0);
        t.performance_by_index = vec![false; len];
        for s in group0 {
            t.performance_by_index[s.logical_index as usize] = s.efficiency_class == max;
        }
        t
    }

    pub fn is_hybrid(&self) -> bool {
        !self.performance.is_empty() && !self.efficiency.is_empty()
    }

    /// 某个核心集合对应的 CPU Set ID；None 表示不限制。
    pub fn ids_for(&self, set: CoreSet) -> Option<&[u32]> {
        if !self.is_hybrid() {
            return None;
        }
        match set {
            CoreSet::All => None,
            CoreSet::Performance => Some(&self.performance),
            CoreSet::Efficiency => Some(&self.efficiency),
        }
    }
}

fn read_cpu_sets() -> Vec<CpuSetEntry> {
    let mut len = 0u32;
    // SAFETY: 先询问所需大小。
    let _ =
        unsafe { GetSystemCpuSetInformation(None, 0, &mut len, Some(GetCurrentProcess()), None) };
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节。
    let ok = unsafe {
        GetSystemCpuSetInformation(
            Some(buf.as_mut_ptr() as *mut SYSTEM_CPU_SET_INFORMATION),
            len,
            &mut len,
            Some(GetCurrentProcess()),
            None,
        )
    };
    if !ok.as_bool() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let base = buf.as_ptr() as *const u8;
    let mut offset = 0usize;
    while offset < len as usize {
        // SAFETY: 每个条目以 Size 字段给出自身长度，始终在 len 范围内。
        let entry = unsafe { &*(base.add(offset) as *const SYSTEM_CPU_SET_INFORMATION) };
        if entry.Type == CpuSetInformation {
            // SAFETY: Type 为 CpuSetInformation 时联合体的 CpuSet 成员有效。
            let cs = unsafe { entry.Anonymous.CpuSet };
            out.push(CpuSetEntry {
                id: cs.Id,
                group: cs.Group,
                logical_index: cs.LogicalProcessorIndex,
                core_index: cs.CoreIndex,
                efficiency_class: cs.EfficiencyClass,
            });
        }
        if entry.Size == 0 {
            break;
        }
        offset += entry.Size as usize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, core: u8, class: u8) -> CpuSetEntry {
        CpuSetEntry {
            id,
            group: 0,
            logical_index: id as u8,
            core_index: core,
            efficiency_class: class,
        }
    }

    #[test]
    fn detects_hybrid_layout_on_this_machine() {
        let t = CpuTopology::detect();
        // 开发机为 i7-12700H（6P+8E，20 线程）；其他机器上至少不应崩溃。
        if t.is_hybrid() {
            assert_eq!(
                t.performance.len() + t.efficiency.len(),
                crate::observe::system::logical_cpus() as usize
            );
            assert!(t.ids_for(CoreSet::All).is_none());
            assert_eq!(
                t.performance_by_index.iter().filter(|&&p| p).count(),
                t.performance.len()
            );
        }
    }

    #[test]
    fn two_classes_count_cores_and_threads() {
        // 2 个大核（各 2 线程）+ 2 个小核。
        let t = CpuTopology::from_sets(&[
            entry(0, 0, 1),
            entry(1, 0, 1),
            entry(2, 1, 1),
            entry(3, 1, 1),
            entry(4, 2, 0),
            entry(5, 3, 0),
        ]);
        assert_eq!(
            t.performance_count,
            CoreCount {
                cores: 2,
                threads: 4
            }
        );
        assert_eq!(
            t.efficiency_count,
            CoreCount {
                cores: 2,
                threads: 2
            }
        );
        assert_eq!(
            t.performance_by_index,
            vec![true, true, true, true, false, false]
        );
    }

    #[test]
    fn three_classes_treat_all_lower_classes_as_small_cores() {
        let t = CpuTopology::from_sets(&[entry(0, 0, 2), entry(1, 1, 1), entry(2, 2, 0)]);
        assert_eq!(t.performance, vec![0]);
        assert_eq!(t.efficiency, vec![1, 2]);
    }

    #[test]
    fn single_class_is_not_hybrid() {
        let t = CpuTopology::from_sets(&[
            entry(0, 0, 0),
            entry(1, 0, 0),
            entry(2, 1, 0),
            entry(3, 1, 0),
        ]);
        assert!(!t.is_hybrid());
        assert_eq!(
            t.performance_count,
            CoreCount {
                cores: 2,
                threads: 4
            }
        );
        assert_eq!(t.efficiency_count, CoreCount::default());
        assert!(t.ids_for(CoreSet::Efficiency).is_none());
    }
}
