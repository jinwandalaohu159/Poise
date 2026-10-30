//! 调度参数与处理器的大小核构成。

use super::trackers::{BusyRule, DemandRule, FocusRule, ForegroundHold, LoadSmoothing};
use crate::action::CoreSet;
use crate::model::Millis;

const MB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// 失去前台后多久内仍算"正在用"、最多保持几个程序。
    pub foreground_hold: ForegroundHold,
    /// 程序负载的平滑时间常数（见 `trackers::LoadSmoothing`）。
    pub load_smoothing: LoadSmoothing,
    /// 什么情况下算有人需要处理器（没人需要时后台不让路）。
    pub demand: DemandRule,
    /// 后台程序怎样算"在忙"、要让路。
    pub busy: BusyRule,
    /// 有重点程序时，发声的程序怎样算"在忙"、要让到小核。门槛高得多：只有真的很忙才值得挪，
    /// 放着歌的浏览器这类轻负载不动（切来切去也不会跟着来回调整）。
    pub busy_with_audio: BusyRule,
    /// 什么样的程序算重点程序（大核留给它，其余没在用的程序让到小核，见 `trackers::focus`）。
    pub focus: FocusRule,
    /// 最后一个游戏退出后，游戏模式再保持多久（一局结束到下一局开始之间不切回日常模式）。
    pub game_session_hold_ms: Millis,
    /// 停声后多久内仍算"在发声"。
    pub audio_hold_ms: Millis,
    /// 自动识别的游戏（全屏且在渲染）要持续多久才确认为"正在玩"。
    pub game_confirm_ms: Millis,
    /// 处理器的大小核构成。
    pub cores: CoreLayout,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            foreground_hold: ForegroundHold {
                ms: 5 * 60_000,
                max_apps: 3,
            },
            load_smoothing: LoadSmoothing {
                // 判断后台在不在忙：短于半分钟的尖峰影响很小。
                slow_ms: 30_000,
                // 判断正在用的程序是不是在干重活：单线程跑满 10 秒内就能认出来。
                fast_ms: 5_000,
            },
            demand: DemandRule {
                in_use_core_pct: 25.0,
                system_pct: 50.0,
                in_use_active_pct: 5.0,
                hold_ms: 30_000,
            },
            busy: BusyRule {
                core_pct: 10.0,
                core_pct_high: 3.0,
                disk_bytes_per_s: 8.0 * MB,
                release_after_ms: 60_000,
                reyield_after_ms: 20_000,
            },
            busy_with_audio: BusyRule {
                core_pct: 25.0,
                core_pct_high: 25.0,
                // 发声的程序（播放、通话）本来就会读写，只看处理器。
                disk_bytes_per_s: f64::INFINITY,
                release_after_ms: 60_000,
                reyield_after_ms: 20_000,
            },
            focus: FocusRule {
                // 持续占用接近一个核心：编译、渲染、导出、游戏这类工作；浏览网页、看视频远低于此。
                enter_core_pct: 80.0,
                leave_core_pct: 40.0,
                hold_ms: 20_000,
            },
            game_session_hold_ms: 3 * 60_000,
            audio_hold_ms: 30_000,
            game_confirm_ms: 6_000,
            // 开发机（i7-12700H）：大核 12 线程，小核两组各 4 线程。实际运行时按检测到的拓扑。
            cores: CoreLayout::new(&[12, 4, 4]),
        }
    }
}

/// 核心域最多这么多个（[`CoreSet::Domains`] 的位数）。
const MAX_DOMAINS: usize = 16;

/// 处理器的核心域（见 [`CoreSet`]）：各个域的线程数，按适合跑重点程序的程度排序。
/// 只有一个域时不限制核心（只用优先级和节能区分前后台）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreLayout {
    threads: [u16; MAX_DOMAINS],
    count: u8,
}

impl CoreLayout {
    /// 副域至少要有这么多线程才把程序让过去：太少时一堆程序挤在一两个核心上互相拖慢，
    /// 只用优先级和节能让路就够了。
    const MIN_AWAY_THREADS: u32 = 4;

    /// 各个域的线程数（排序见 [`CoreSet`]）。超过 16 个域时，多出来的并入最后一个。
    pub fn new(domain_threads: &[u32]) -> Self {
        let mut threads = [0u16; MAX_DOMAINS];
        for (i, &t) in domain_threads.iter().enumerate() {
            let slot = i.min(MAX_DOMAINS - 1);
            threads[slot] = threads[slot].saturating_add(t.min(u32::from(u16::MAX)) as u16);
        }
        Self {
            threads,
            count: domain_threads.len().min(MAX_DOMAINS) as u8,
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

    /// 能不能把重点程序限制在主域上：至少两个域。
    pub fn partitioned(&self) -> bool {
        self.count >= 2
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

    /// 由窄到宽的核心范围及其线程数：主域，主域加第一个副域……直到全部核心。只有一个域时只有"不限制"。
    pub fn tiers(&self) -> Vec<(CoreSet, u32)> {
        let mut tiers = Vec::with_capacity(usize::from(self.count));
        let mut threads = 0;
        for d in 0..self.count {
            threads += self.threads(d);
            let set = if d + 1 == self.count {
                CoreSet::All
            } else {
                CoreSet::Domains(((1u32 << (d + 1)) - 1) as u16)
            };
            tiers.push((set, threads));
        }
        if tiers.is_empty() {
            tiers.push((CoreSet::All, 0));
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
                (CoreSet::Domains(0b1), 12),
                (CoreSet::Domains(0b11), 16),
                (CoreSet::All, 20)
            ]
        );
        assert!(alder.can_move_away());
        assert!(alder.allows(CoreSet::Domains(0b100)));
        assert!(!alder.allows(CoreSet::Domains(0b1000)));
        // 只有一个域：不限制。
        let uniform = CoreLayout::uniform(16);
        assert_eq!(uniform.tiers(), vec![(CoreSet::All, 16)]);
        assert!(!uniform.allows(CoreSet::Performance) && !uniform.allows(CoreSet::Efficiency));
        // 副域太小：重点程序可以放在主域，别的程序不让过去。
        let few = CoreLayout::new(&[8, 2]);
        assert!(few.allows(CoreSet::Performance) && !few.allows(CoreSet::Efficiency));
    }

    #[test]
    fn extra_domains_are_merged_into_the_last() {
        let many: Vec<u32> = vec![2; 20];
        let l = CoreLayout::new(&many);
        assert_eq!(l.domains(), 16);
        assert_eq!(l.total_threads(), 40);
        assert_eq!(l.threads(15), 10);
    }
}
