//! CPU 拓扑：大小核与核心域。都从系统报告的信息自动得出，不写死核心编号，也不按处理器型号区分。
//!
//! - 大小核：按 EfficiencyClass 区分。效率等级最高的是大核，其余全部算小核（兼容有两种以上小核的处理器）。
//!   电源设置按效率等级写入（见硬件层），界面按它显示核心数量。
//! - 核心域（见 [`CoreSet`]）：效率等级相同、共用一块缓存的核心。同一个程序放在同一个域里，线程之间共享的
//!   数据留在同一块缓存里，不在缓存之间来回搬。先按最后一级缓存分（多 CCD 的锐龙每个 CCD 一块三级缓存），
//!   每块里再按二级缓存分——但只在每组都至少有 [`domains::MIN_DOMAIN_THREADS`] 个线程时才分（酷睿的小核每 4 个共用
//!   一块二级缓存，要分；大核每个核心独占一块，分开就成了两线程一组，没有意义）。
//!
//! 核心域按适合跑重点程序的程度排序：效率等级高的在前；同等级里缓存大的在前（带 3D 缓存的 CCD）；
//! 再按编号。例如 12 代酷睿：[大核 12 线程] [小核组 0] [小核组 1]；双 CCD 锐龙：[CCD0] [CCD1]；
//! 只有一块缓存的普通处理器：一个域，不限制核心。

mod domains;
mod read;

use std::collections::HashSet;

use poise_core::action::CoreSet;

use domains::domains;
use read::{CacheEntry, CpuSetEntry, read_caches, read_cpu_sets};

/// 一类核心的数量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoreCount {
    pub cores: u32,
    pub threads: u32,
}

/// 一个逻辑处理器：处理器组、组内序号，以及属于组内的哪个物理核心（超线程的两个逻辑处理器同属一个核心）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogicalProcessor {
    pub group: u16,
    pub index: u8,
    pub core: u8,
}

/// 一个核心域：哪些逻辑处理器、几个物理核心、是不是大核，以及缓存（界面的核心与缓存面板按它画）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreDomain {
    /// CPU Set ID（限制进程用哪些核心时用）。
    pub ids: Vec<u32>,
    /// 逻辑处理器，与 `ids` 一一对应。
    pub processors: Vec<LogicalProcessor>,
    /// 物理核心数。
    pub cores: u32,
    /// 效率等级最高的核心（大核；不是大小核架构时全部是）。
    pub performance: bool,
    /// 二级缓存每块的容量，以及每块由几个逻辑处理器共用；读不到时为 0。
    pub l2_bytes: u32,
    pub l2_threads: u32,
    /// 所在最后一级缓存的容量（读不到时为 0）和编号：几个域共用同一块时编号相同。
    pub llc_bytes: u32,
    pub llc_block: Option<u8>,
    /// 物理核心（处理器组，组内核心编号），系统优先使用的在前（睿频最高的"优选核心"，见
    /// `CpuSetEntry::scheduling_class`），同等的按编号。主域按物理核心切开时，前面的留给重点程序。
    pub preferred_cores: Vec<(u16, u8)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuTopology {
    /// 大核数量；不是大小核架构时是全部核心。
    pub performance_count: CoreCount,
    /// 小核数量；不是大小核架构时为 0。
    pub efficiency_count: CoreCount,
    /// 第 0 组各逻辑处理器是否是大核（按逻辑处理器序号）。不是大小核架构时为空。
    pub performance_by_index: Vec<bool>,
    /// 最高的效率等级（大核所在的等级）。Windows 从 0（最省电的核心）开始编号，不是大小核架构时为 0。
    pub top_class: u8,
    /// 核心域，排序见模块说明。只有一个域时不限制核心。
    pub domains: Vec<CoreDomain>,
}

impl CpuTopology {
    pub fn detect() -> Self {
        Self::from_sets(&read_cpu_sets(), &read_caches())
    }

    fn from_sets(sets: &[CpuSetEntry], caches: &[CacheEntry]) -> Self {
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
            top_class: max,
            domains: domains(sets, caches),
            ..Self::default()
        };
        if !small.is_empty() {
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
        }
        t
    }

    /// 有大小核之分。
    pub fn is_hybrid(&self) -> bool {
        self.performance_count.threads > 0 && self.efficiency_count.threads > 0
    }

    /// 大核的几个核心域最后一级缓存大小不同（例如 7950X3D 这样一组带 3D 缓存的双 CCD 处理器）。
    pub fn uneven_cache(&self) -> bool {
        let sizes: HashSet<u32> = self
            .domains
            .iter()
            .filter(|d| d.performance && d.llc_bytes > 0)
            .map(|d| d.llc_bytes)
            .collect();
        sizes.len() > 1
    }

    /// 能不能把进程限制在一部分核心上：至少有两个核心域。
    pub fn partitioned(&self) -> bool {
        self.domains.len() >= 2
    }

    /// 全部逻辑处理器的 CPU Set ID。
    pub fn all_ids(&self) -> Vec<u32> {
        self.domains
            .iter()
            .flat_map(|d| d.ids.iter().copied())
            .collect()
    }

    /// 副域和主域一样快：都是效率等级最高的核心，只是各有一块缓存（多 CCD 的锐龙）。大小核处理器的副域是小核。
    pub fn away_equal(&self) -> bool {
        self.partitioned() && self.domains[1..].iter().all(|d| d.performance)
    }

    /// 主域各物理核心（按系统优先使用的顺序，见 [`CoreDomain::preferred_cores`]）在第 0 组里的逻辑处理器序号：
    /// 看每个物理核心有多忙用。处理器结构不变，调用方建一次留着。
    pub fn preferred_core_threads(&self) -> Vec<Vec<usize>> {
        let Some(main) = self.domains.first() else {
            return Vec::new();
        };
        main.preferred_cores
            .iter()
            .map(|&(group, core)| {
                main.processors
                    .iter()
                    .filter(|p| p.group == 0 && p.group == group && p.core == core)
                    .map(|p| usize::from(p.index))
                    .collect()
            })
            .collect()
    }

    /// 各核心域的（线程数，物理核心数）（调度按它分配，见 `CoreLayout`）。
    pub fn domain_sizes(&self) -> Vec<(u32, u32)> {
        self.domains
            .iter()
            .map(|d| (d.ids.len() as u32, d.cores))
            .collect()
    }

    /// 某个核心集合对应的 CPU Set ID；None 表示不限制（包括覆盖了全部核心域）。
    pub fn ids_for(&self, set: CoreSet) -> Option<Vec<u32>> {
        match set {
            CoreSet::Rest(kept) => return self.ids_except_preferred(kept),
            CoreSet::Preferred(kept) => return self.ids_of_preferred(kept),
            _ => {}
        }
        if !self.partitioned() {
            return None;
        }
        let count = self.domains.len();
        let mask = set.domain_mask(count);
        if mask == CoreSet::All.domain_mask(count) {
            return None;
        }
        Some(
            self.domains
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .flat_map(|(_, d)| d.ids.iter().copied())
                .collect(),
        )
    }

    /// 除了主域里系统优先使用的前 `kept` 个物理核心（两个超线程一起）之外的全部核心（见 [`CoreSet::Rest`]）。
    fn ids_except_preferred(&self, kept: u8) -> Option<Vec<u32>> {
        let (_, main_rest) = self.split_main(kept)?;
        let others = self.domains[1..].iter().flat_map(|d| d.ids.iter().copied());
        // 什么都不剩时不限制（空的核心集合在系统里表示"不限制"，这里明确写出来）。
        let rest: Vec<u32> = main_rest.into_iter().chain(others).collect();
        (!rest.is_empty()).then_some(rest)
    }

    /// 主域里系统优先使用的前 `kept` 个物理核心（两个超线程一起，见 [`CoreSet::Preferred`]）。切不开
    /// （留的不少于主域的全部核心）时按整个主域：只有一个域时不限制。
    fn ids_of_preferred(&self, kept: u8) -> Option<Vec<u32>> {
        match self.split_main(kept) {
            Some((preferred, rest)) if !preferred.is_empty() && !rest.is_empty() => Some(preferred),
            _ => self.ids_for(CoreSet::Performance),
        }
    }

    /// 主域按物理核心切开：（系统优先使用的前 `kept` 个物理核心，其余）的 CPU Set ID。
    fn split_main(&self, kept: u8) -> Option<(Vec<u32>, Vec<u32>)> {
        let main = self.domains.first()?;
        if kept == 0 {
            return None;
        }
        let kept: HashSet<(u16, u8)> = main
            .preferred_cores
            .iter()
            .take(usize::from(kept))
            .copied()
            .collect();
        let (preferred, rest): (Vec<_>, Vec<_>) = main
            .ids
            .iter()
            .zip(&main.processors)
            .partition(|(_, p)| kept.contains(&(p.group, p.core)));
        let ids = |v: Vec<(&u32, _)>| v.into_iter().map(|(id, _)| *id).collect();
        Some((ids(preferred), ids(rest)))
    }
}

#[cfg(test)]
mod tests;
