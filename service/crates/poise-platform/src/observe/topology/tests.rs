use super::*;

/// 第 0 组的逻辑处理器 `id`，属于物理核心 `core`。
fn entry(id: u32, core: u8, class: u8) -> CpuSetEntry {
    CpuSetEntry {
        id,
        group: 0,
        logical_index: id as u8,
        core_index: core,
        efficiency_class: class,
        scheduling_class: 0,
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
    // 小核比大核慢：大核按物理核心切开分享。
    assert!(!t.away_equal());
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
fn the_rest_leaves_out_the_preferred_big_cores_with_both_threads() {
    let (mut sets, caches) = alder_lake();
    // 第二个大核是系统优先使用的（睿频最高）。
    for s in sets.iter_mut().filter(|s| s.core_index == 1) {
        s.scheduling_class = 2;
    }
    let t = CpuTopology::from_sets(&sets, &caches);
    assert_eq!(t.domains[0].preferred_cores, vec![(0, 1), (0, 0)]);
    assert_eq!(t.preferred_core_threads(), vec![vec![2, 3], vec![0, 1]]);
    assert_eq!(
        t.ids_for(CoreSet::Rest(1)),
        Some(vec![0, 1, 4, 5, 6, 7, 8, 9, 10, 11])
    );
    assert_eq!(t.ids_for(CoreSet::Rest(2)), Some((4..12).collect()));
    assert_eq!(t.ids_for(CoreSet::Rest(0)), None);
    // 另一半：重点程序用优选的那个物理核心，两个超线程一起，和其余核心不重叠。
    assert_eq!(t.ids_for(CoreSet::Preferred(1)), Some(vec![2, 3]));
    // 切不开（留下整个主域）：按整个主域。
    assert_eq!(t.ids_for(CoreSet::Preferred(2)), Some(vec![0, 1, 2, 3]));
    assert_eq!(t.ids_for(CoreSet::Preferred(0)), Some(vec![0, 1, 2, 3]));
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
    // 两个 CCD 一样快：重点程序留着整个 CCD，别的程序用另一个。
    assert!(t.away_equal());
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
        assert_eq!(t.domain_sizes(), vec![(12, 6), (4, 4), (4, 4)]);
    }
}
