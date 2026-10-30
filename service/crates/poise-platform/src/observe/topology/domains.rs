//! 划分核心域（规则见上级模块的说明）。纯计算：输入是系统报告的 CPU Set 与缓存，输出按排序的各域。

use std::collections::BTreeMap;

use super::read::{CacheEntry, CpuSetEntry};

/// 按二级缓存分出的核心域至少要有这么多线程。
pub(super) const MIN_DOMAIN_THREADS: usize = 4;
/// 核心域最多这么多个（[`poise_core::action::CoreSet::Domains`] 的位数）；更多时最后几个合在一起。
pub(super) const MAX_DOMAINS: usize = 16;

/// 一个核心域在排序时看的：效率等级、所在最后一级缓存的容量、最小的逻辑处理器编号。
struct DomainDraft {
    class: u8,
    llc_size: u32,
    first: (u16, u8),
    ids: Vec<u32>,
}

/// 划分核心域（规则见模块说明）。
pub(super) fn domains(sets: &[CpuSetEntry], caches: &[CacheEntry]) -> Vec<Vec<u32>> {
    let find =
        |level: u8, s: &CpuSetEntry| caches.iter().position(|c| c.level == level && c.covers(s));
    let mut drafts = Vec::new();
    // 按（效率等级，最后一级缓存）分组；读不到三级缓存时同一等级算一块。
    let mut by_llc: BTreeMap<(u8, Option<usize>), Vec<&CpuSetEntry>> = BTreeMap::new();
    for s in sets {
        by_llc
            .entry((s.efficiency_class, find(3, s)))
            .or_default()
            .push(s);
    }
    for ((class, llc), members) in by_llc {
        let llc_size = llc.map_or(0, |i| caches[i].size);
        let mut by_l2: BTreeMap<Option<usize>, Vec<&CpuSetEntry>> = BTreeMap::new();
        for s in &members {
            by_l2.entry(find(2, s)).or_default().push(s);
        }
        let split = by_l2.len() >= 2
            && by_l2
                .iter()
                .all(|(l2, m)| l2.is_some() && m.len() >= MIN_DOMAIN_THREADS);
        let parts: Vec<Vec<&CpuSetEntry>> = if split {
            by_l2.into_values().collect()
        } else {
            vec![members]
        };
        for part in parts {
            drafts.push(DomainDraft {
                class,
                llc_size,
                first: part
                    .iter()
                    .map(|s| (s.group, s.logical_index))
                    .min()
                    .unwrap_or_default(),
                ids: part.iter().map(|s| s.id).collect(),
            });
        }
    }
    drafts.sort_by(|a, b| {
        b.class
            .cmp(&a.class)
            .then(b.llc_size.cmp(&a.llc_size))
            .then(a.first.cmp(&b.first))
    });
    let mut out: Vec<Vec<u32>> = drafts.into_iter().map(|d| d.ids).collect();
    if out.len() > MAX_DOMAINS {
        let rest: Vec<u32> = out.drain(MAX_DOMAINS - 1..).flatten().collect();
        out.push(rest);
    }
    out
}
