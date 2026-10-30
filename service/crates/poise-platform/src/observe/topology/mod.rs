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

    /// 各核心域的线程数（调度按它分配，见 `CoreLayout`）。
    pub fn domain_threads(&self) -> Vec<u32> {
        self.domains.iter().map(|d| d.ids.len() as u32).collect()
    }

    /// 某个核心集合对应的 CPU Set ID；None 表示不限制（包括覆盖了全部核心域）。
    pub fn ids_for(&self, set: CoreSet) -> Option<Vec<u32>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 第 0 组的逻辑处理器 `id`，属于物理核心 `core`。
    fn entry(id: u32, core: u8, class: u8) -> CpuSetEntry {
        CpuSetEntry {
            id,
            group: 0,
            logical_index: id as u8,
            core_index: core,
            efficiency_class: class,
        }
    }

    fn ids(t: &CpuTopology) -> Vec<Vec<u32>> {
        t.domains.iter().map(|d| d.ids.clone()).collect()
    }

    fn cache(level: u8, size: u32, ids: std::ops::Range<u32>) -> CacheEntry {
        CacheEntry {
            level,
            size,
            group: 0,
            mask: ids.fold(0, |m, i| m | (1 << i)),
        }
    }

    /// 12 代酷睿的缩小版：2 个大核（各 2 线程，各自一块二级缓存）+ 8 个小核（每 4 个共用一块二级缓存），
    /// 全部共用一块三级缓存。
    fn alder_lake() -> (Vec<CpuSetEntry>, Vec<CacheEntry>) {
        let mut sets = vec![
            entry(0, 0, 1),
            entry(1, 0, 1),
            entry(2, 1, 1),
            entry(3, 1, 1),
        ];
        sets.extend((4..12).map(|i| entry(i, i as u8, 0)));
        let caches = vec![
            cache(2, 1 << 20, 0..2),
            cache(2, 1 << 20, 2..4),
            cache(2, 2 << 20, 4..8),
            cache(2, 2 << 20, 8..12),
            cache(3, 24 << 20, 0..12),
        ];
        (sets, caches)
    }

    #[test]
    fn hybrid_cores_split_into_the_big_cores_and_small_core_clusters() {
        let (sets, caches) = alder_lake();
        let t = CpuTopology::from_sets(&sets, &caches);
        assert_eq!(
            ids(&t),
            vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7], vec![8, 9, 10, 11]]
        );
        assert!(t.is_hybrid() && t.partitioned());
        assert_eq!(t.ids_for(CoreSet::Performance), Some(vec![0, 1, 2, 3]));
        assert_eq!(t.ids_for(CoreSet::Efficiency), Some((4..12).collect()));
        assert_eq!(t.ids_for(CoreSet::Domains(0b100)), Some(vec![8, 9, 10, 11]));
        assert_eq!(
            t.ids_for(CoreSet::Domains(0b011)),
            Some(vec![0, 1, 2, 3, 4, 5, 6, 7])
        );
        // 覆盖全部核心域：等于不限制。
        assert_eq!(t.ids_for(CoreSet::Domains(0b111)), None);
        assert_eq!(t.ids_for(CoreSet::All), None);
        // 读不到缓存信息：大核、小核各一个域。
        let t = CpuTopology::from_sets(&sets, &[]);
        assert_eq!(ids(&t), vec![vec![0, 1, 2, 3], (4..12).collect::<Vec<_>>()]);
    }

    #[test]
    fn domains_describe_their_cores_and_caches() {
        let (sets, caches) = alder_lake();
        let t = CpuTopology::from_sets(&sets, &caches);
        let [big, small, _] = t.domains.as_slice() else {
            panic!("应有三个域");
        };
        assert!(big.performance && !small.performance);
        assert_eq!((big.cores, small.cores), (2, 4));
        assert_eq!(
            big.processors
                .iter()
                .map(|p| (p.index, p.core))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 0), (2, 1), (3, 1)]
        );
        // 大核每个核心一块二级缓存（2 线程共用），小核 4 个共用一块。
        assert_eq!((big.l2_bytes, big.l2_threads), (1 << 20, 2));
        assert_eq!((small.l2_bytes, small.l2_threads), (2 << 20, 4));
        // 三级缓存全部共用：编号相同。
        assert!(
            t.domains
                .iter()
                .all(|d| d.llc_bytes == 24 << 20 && d.llc_block == Some(0))
        );
    }

    #[test]
    fn multi_ccd_processors_split_by_last_level_cache_and_prefer_the_bigger_one() {
        // 双 CCD、每个 CCD 2 个核心（各 2 线程），第二个 CCD 带 3D 缓存。
        let sets: Vec<CpuSetEntry> = (0..8).map(|i| entry(i, (i / 2) as u8, 0)).collect();
        let caches = vec![
            cache(2, 1 << 20, 0..2),
            cache(2, 1 << 20, 2..4),
            cache(2, 1 << 20, 4..6),
            cache(2, 1 << 20, 6..8),
            cache(3, 32 << 20, 0..4),
            cache(3, 96 << 20, 4..8),
        ];
        let t = CpuTopology::from_sets(&sets, &caches);
        assert!(!t.is_hybrid());
        assert_eq!(ids(&t), vec![vec![4, 5, 6, 7], vec![0, 1, 2, 3]]);
        assert_eq!(t.ids_for(CoreSet::Efficiency), Some(vec![0, 1, 2, 3]));
        // 两块三级缓存各自编号，带 3D 缓存的在前。
        assert_eq!(
            t.domains
                .iter()
                .map(|d| (d.llc_bytes >> 20, d.llc_block))
                .collect::<Vec<_>>(),
            vec![(96, Some(0)), (32, Some(1))]
        );
    }

    #[test]
    fn a_single_cache_means_one_domain_and_no_restriction() {
        let sets: Vec<CpuSetEntry> = (0..4).map(|i| entry(i, (i / 2) as u8, 0)).collect();
        let t = CpuTopology::from_sets(&sets, &[cache(3, 8 << 20, 0..4)]);
        assert_eq!(t.domains.len(), 1);
        assert!(!t.partitioned() && !t.is_hybrid());
        assert!(t.ids_for(CoreSet::Efficiency).is_none());
        assert!(t.ids_for(CoreSet::Performance).is_none());
        assert_eq!(
            t.performance_count,
            CoreCount {
                cores: 2,
                threads: 4
            }
        );
        assert_eq!(t.efficiency_count, CoreCount::default());
    }

    #[test]
    fn three_classes_make_three_domains_from_fast_to_slow() {
        let t = CpuTopology::from_sets(&[entry(0, 0, 2), entry(1, 1, 1), entry(2, 2, 0)], &[]);
        assert_eq!(ids(&t), vec![vec![0], vec![1], vec![2]]);
        assert_eq!(t.ids_for(CoreSet::Efficiency), Some(vec![1, 2]));
        assert_eq!(t.performance_count.threads, 1);
        assert_eq!(t.efficiency_count.threads, 2);
    }

    #[test]
    fn two_classes_count_cores_and_threads() {
        let (sets, caches) = alder_lake();
        let t = CpuTopology::from_sets(&sets, &caches);
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
                cores: 8,
                threads: 8
            }
        );
        assert_eq!(t.performance_by_index.iter().filter(|&&p| p).count(), 4);
    }

    #[test]
    fn too_many_domains_are_merged_at_the_tail() {
        let sets: Vec<CpuSetEntry> = (0..40).map(|i| entry(i, i as u8, 0)).collect();
        let caches: Vec<CacheEntry> = (0..20)
            .map(|i| cache(3, 1 << 20, i * 2..i * 2 + 2))
            .collect();
        let t = CpuTopology::from_sets(&sets, &caches);
        assert_eq!(t.domains.len(), domains::MAX_DOMAINS);
        assert_eq!(t.domains.iter().map(|d| d.ids.len()).sum::<usize>(), 40);
    }

    #[test]
    fn detects_this_machine() {
        let t = CpuTopology::detect();
        // 每个逻辑处理器恰好在一个核心域里；其他机器上至少不应崩溃。
        let total: usize = t.domains.iter().map(|d| d.ids.len()).sum();
        assert_eq!(total, crate::observe::system::logical_cpus() as usize);
        // 开发机为 i7-12700H（6P+8E，20 线程）：大核一个域，小核按二级缓存分两组。
        if t.performance_count.threads == 12 && t.efficiency_count.threads == 8 {
            assert_eq!(t.domain_threads(), vec![12, 4, 4]);
        }
    }
}
