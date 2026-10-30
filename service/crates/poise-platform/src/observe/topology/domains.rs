//! 划分核心域（规则见上级模块的说明）。纯计算：输入是系统报告的 CPU Set 与缓存，输出按排序的各域。

use std::collections::{BTreeMap, HashSet};

use super::read::{CacheEntry, CpuSetEntry};
use super::{CoreDomain, LogicalProcessor};

/// 按二级缓存分出的核心域至少要有这么多线程。
pub(super) const MIN_DOMAIN_THREADS: usize = 4;
/// 核心域最多这么多个（[`poise_core::action::CoreSet::Domains`] 的位数）；更多时最后几个合在一起。
pub(super) const MAX_DOMAINS: usize = 16;

/// 一个核心域在排序时看的：效率等级、所在最后一级缓存的容量、最小的逻辑处理器编号。
struct DomainDraft<'a> {
    class: u8,
    llc: Option<usize>,
    llc_size: u32,
    first: (u16, u8),
    members: Vec<&'a CpuSetEntry>,
}

/// 划分核心域（规则见模块说明）。
pub(super) fn domains(sets: &[CpuSetEntry], caches: &[CacheEntry]) -> Vec<CoreDomain> {
    let find =
        |level: u8, s: &CpuSetEntry| caches.iter().position(|c| c.level == level && c.covers(s));
    let top_class = sets.iter().map(|s| s.efficiency_class).max().unwrap_or(0);
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
                llc,
                llc_size,
                first: part
                    .iter()
                    .map(|s| (s.group, s.logical_index))
                    .min()
                    .unwrap_or_default(),
                members: part,
            });
        }
    }
    drafts.sort_by(|a, b| {
        b.class
            .cmp(&a.class)
            .then(b.llc_size.cmp(&a.llc_size))
            .then(a.first.cmp(&b.first))
    });
    if drafts.len() > MAX_DOMAINS {
        let rest: Vec<&CpuSetEntry> = drafts
            .drain(MAX_DOMAINS - 1..)
            .flat_map(|d| d.members)
            .collect();
        let tail = drafts
            .last()
            .map_or((0, None, 0), |d| (d.class, d.llc, d.llc_size));
        drafts.push(DomainDraft {
            class: tail.0,
            llc: tail.1,
            llc_size: tail.2,
            first: (0, 0),
            members: rest,
        });
    }
    // 最后一级缓存按出现的先后编号：几个域共用同一块时编号相同。
    let mut llc_numbers: Vec<usize> = Vec::new();
    drafts
        .into_iter()
        .map(|d| {
            let llc_block = d.llc.map(|i| {
                let n = llc_numbers.iter().position(|&x| x == i).unwrap_or_else(|| {
                    llc_numbers.push(i);
                    llc_numbers.len() - 1
                });
                n as u8
            });
            describe(&d, top_class, llc_block, caches)
        })
        .collect()
}

/// 把一个划分好的域写成对外的描述：逻辑处理器、核心数、二级缓存。
fn describe(
    d: &DomainDraft,
    top_class: u8,
    llc_block: Option<u8>,
    caches: &[CacheEntry],
) -> CoreDomain {
    let mut members = d.members.clone();
    members.sort_by_key(|s| (s.group, s.logical_index));
    // 二级缓存：取域里第一个逻辑处理器所在的那块（同一个域里的各块一样大）。
    let l2 = members
        .first()
        .and_then(|s| caches.iter().find(|c| c.level == 2 && c.covers(s)));
    CoreDomain {
        ids: members.iter().map(|s| s.id).collect(),
        processors: members
            .iter()
            .map(|s| LogicalProcessor {
                group: s.group,
                index: s.logical_index,
                core: s.core_index,
            })
            .collect(),
        cores: members
            .iter()
            .map(|s| (s.group, s.core_index))
            .collect::<HashSet<_>>()
            .len() as u32,
        performance: d.class == top_class,
        l2_bytes: l2.map_or(0, |c| c.size),
        l2_threads: l2.map_or(0, |c| c.mask.count_ones()),
        llc_bytes: d.llc_size,
        llc_block,
    }
}
