//! 处理器的核心域（见 [`CoreSet`]）：各个域有几个线程、几个物理核心，按适合跑重点程序的程度排序。

use crate::action::CoreSet;

/// 核心域最多这么多个（[`CoreSet::Domains`] 的位数）。
const MAX_DOMAINS: usize = 16;

/// 处理器的核心域。只有一个域时不限制核心（只用优先级和节能区分前后台）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreLayout {
    threads: [u16; MAX_DOMAINS],
    cores: [u16; MAX_DOMAINS],
    count: u8,
}

impl CoreLayout {
    /// 副域至少要有这么多线程才把程序让过去：太少时一堆程序挤在一两个核心上互相拖慢，
    /// 只用优先级和节能让路就够了。
    const MIN_AWAY_THREADS: u32 = 4;
    /// 超线程多出来的那个线程大约能多做这么多（相对一个物理核心）：两个忙线程挤在同一个物理核心上，
    /// 合起来只比一个快四分之一左右。
    const SMT_YIELD: f32 = 0.25;
    /// 主域至少有这么多物理核心……
    const CONFINE_MIN_CORES: u32 = 6;
    /// ……或者至少占整机能力的这个比例，才把重点程序限制在主域上。
    const CONFINE_MIN_SHARE: f32 = 0.4;

    /// 各个域的线程数，每个线程都是一个物理核心（没有超线程）。
    pub fn new(domain_threads: &[u32]) -> Self {
        let domains: Vec<(u32, u32)> = domain_threads.iter().map(|&t| (t, t)).collect();
        Self::with_cores(&domains)
    }

    /// 各个域的（线程数，物理核心数），排序见 [`CoreSet`]。超过 16 个域时，多出来的并入最后一个。
    pub fn with_cores(domains: &[(u32, u32)]) -> Self {
        let mut threads = [0u16; MAX_DOMAINS];
        let mut cores = [0u16; MAX_DOMAINS];
        let narrow = |n: u32| n.min(u32::from(u16::MAX)) as u16;
        for (i, &(t, c)) in domains.iter().enumerate() {
            let slot = i.min(MAX_DOMAINS - 1);
            threads[slot] = threads[slot].saturating_add(narrow(t));
            cores[slot] = cores[slot].saturating_add(narrow(c.min(t)));
        }
        Self {
            threads,
            cores,
            count: domains.len().min(MAX_DOMAINS) as u8,
        }
    }

    /// 只有一块缓存、效率等级相同的处理器：一个域。
    pub fn uniform(threads: u32) -> Self {
        Self::new(&[threads])
    }

    pub fn domains(&self) -> u8 {
        self.count
    }

    /// 某个域的线程数。
    pub fn threads(&self, domain: u8) -> u32 {
        self.threads
            .get(usize::from(domain))
            .map_or(0, |&t| u32::from(t))
    }

    /// 某个域的物理核心数。
    pub fn cores(&self, domain: u8) -> u32 {
        self.cores
            .get(usize::from(domain))
            .map_or(0, |&c| u32::from(c))
    }

    /// 某个域能承担多少负载（单核百分比）：物理核心各算 100%，超线程多出来的线程各算 25%
    /// （见 [`Self::SMT_YIELD`]）。
    pub fn capacity(&self, domain: u8) -> f32 {
        let (threads, cores) = (self.threads(domain), self.cores(domain));
        (cores as f32 + (threads - cores) as f32 * Self::SMT_YIELD) * 100.0
    }

    pub fn total_capacity(&self) -> f32 {
        (0..self.count).map(|d| self.capacity(d)).sum()
    }

    pub fn total_threads(&self) -> u32 {
        (0..self.count).map(|d| self.threads(d)).sum()
    }

    /// 全部副域的线程数。
    pub fn away_threads(&self) -> u32 {
        self.total_threads() - self.threads(0)
    }

    /// 全部域的位掩码。
    pub fn all_mask(&self) -> u16 {
        ((1u32 << self.count) - 1) as u16
    }

    /// 能不能把进程限制在某些域上：至少两个域。
    pub fn partitioned(&self) -> bool {
        self.count >= 2
    }

    /// 能不能把重点程序限制在主域上：至少两个域，而且主域够大（见 [`Self::CONFINE_MIN_CORES`]、
    /// [`Self::CONFINE_MIN_SHARE`]）。主域只有两三个大核的机器（例如 2 大 8 小的低压处理器）上，
    /// 把游戏关进去比不管还糟：它的工作线程只能挤在大核的超线程上。这时重点程序不限制核心，
    /// 只把别的程序让到副域。
    pub fn can_confine_focus(&self) -> bool {
        self.partitioned()
            && (self.cores(0) >= Self::CONFINE_MIN_CORES
                || self.capacity(0) >= self.total_capacity() * Self::CONFINE_MIN_SHARE)
    }

    /// 能不能把程序让到副域：至少两个域，副域的线程也够多。
    pub fn can_move_away(&self) -> bool {
        self.partitioned() && self.away_threads() >= Self::MIN_AWAY_THREADS
    }

    /// 能不能把核心范围限制到 `set`。
    pub(super) fn allows(&self, set: CoreSet) -> bool {
        let valid = |m: u16| m != 0 && m & !self.all_mask() == 0;
        match set {
            CoreSet::All => true,
            CoreSet::Performance => self.partitioned(),
            CoreSet::Efficiency => self.can_move_away(),
            CoreSet::Domains(m) if set.is_performance() => self.partitioned() && valid(m),
            CoreSet::Domains(m) => self.can_move_away() && valid(m),
        }
    }

    /// 重点程序由窄到宽的核心范围及其能力（单核百分比）：主域，主域加第一个副域……直到全部核心。
    /// 只有一个域、或者主域太小不限制重点程序时（见 [`Self::can_confine_focus`]）只有"不限制"。
    pub fn tiers(&self) -> Vec<(CoreSet, f32)> {
        if !self.can_confine_focus() {
            return vec![(CoreSet::All, self.total_capacity())];
        }
        let mut tiers = Vec::with_capacity(usize::from(self.count));
        let mut capacity = 0.0;
        for d in 0..self.count {
            capacity += self.capacity(d);
            let set = if d + 1 == self.count {
                CoreSet::All
            } else {
                CoreSet::Domains(((1u32 << (d + 1)) - 1) as u16)
            };
            tiers.push((set, capacity));
        }
        tiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_widen_one_domain_at_a_time() {
        let alder = CoreLayout::new(&[12, 4, 4]);
        assert_eq!(
            alder.tiers(),
            vec![
                (CoreSet::Domains(0b1), 1200.0),
                (CoreSet::Domains(0b11), 1600.0),
                (CoreSet::All, 2000.0)
            ]
        );
        assert!(alder.can_move_away());
        assert!(alder.allows(CoreSet::Domains(0b100)));
        assert!(!alder.allows(CoreSet::Domains(0b1000)));
        // 只有一个域：不限制。
        let uniform = CoreLayout::uniform(16);
        assert_eq!(uniform.tiers(), vec![(CoreSet::All, 1600.0)]);
        assert!(!uniform.allows(CoreSet::Performance) && !uniform.allows(CoreSet::Efficiency));
        // 副域太小：重点程序可以放在主域，别的程序不让过去。
        let few = CoreLayout::new(&[8, 2]);
        assert!(few.allows(CoreSet::Performance) && !few.allows(CoreSet::Efficiency));
    }

    #[test]
    fn hyperthreads_count_for_a_quarter_of_a_core() {
        // i7-12700H：6 个大核 12 线程，两组各 4 个小核。
        let h = CoreLayout::with_cores(&[(12, 6), (4, 4), (4, 4)]);
        assert_eq!(h.capacity(0), 750.0);
        assert_eq!(h.total_capacity(), 1550.0);
        assert!(h.can_confine_focus());
    }

    #[test]
    fn a_small_primary_domain_does_not_confine_the_focused_program() {
        // i5-1235U：2 个大核 4 线程，两组各 4 个小核。
        let u = CoreLayout::with_cores(&[(4, 2), (4, 4), (4, 4)]);
        assert!(!u.can_confine_focus());
        assert_eq!(u.tiers(), vec![(CoreSet::All, 1050.0)]);
        // 别的程序照样可以让到小核。
        assert!(u.can_move_away());
        // Lunar Lake：4 个大核、4 个低功耗小核，大核占一半能力。
        assert!(CoreLayout::new(&[4, 4]).can_confine_focus());
    }

    #[test]
    fn extra_domains_are_merged_into_the_last() {
        let many: Vec<u32> = vec![2; 20];
        let l = CoreLayout::new(&many);
        assert_eq!(l.domains(), 16);
        assert_eq!(l.total_threads(), 40);
        assert_eq!(l.threads(15), 10);
        assert_eq!(l.cores(15), 10);
    }
}
