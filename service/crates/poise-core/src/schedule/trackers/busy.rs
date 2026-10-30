//! 后台忙碌：哪些程序（进程树）该让路。

use std::collections::HashMap;
use std::collections::HashSet;

use super::demand::Pressure;
use crate::model::{Millis, ProcessKey};

/// 后台忙碌的判断规则。负载是整棵进程树平滑后的负载（见 `TreeLoad`），短暂的活动本来就不会让它升高。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusyRule {
    /// 需求一般时，负载（单核百分比）达到这么多算在忙。
    pub core_pct: f32,
    /// 需求高时（玩游戏、接近温度上限）的门槛：更低，后台更早让路。
    pub core_pct_high: f32,
    /// 磁盘读写（字节每秒，平滑后）达到这么多也算在忙：处理器占用不高、却一直读写磁盘的后台程序
    /// （同步、下载、备份）同样会拖慢前台和游戏。需求高时门槛减半。
    pub disk_bytes_per_s: f64,
    /// 已经让路的程序，持续这么久没有需求后恢复。
    pub release_after_ms: Millis,
    /// 程序不再"正在用"（离开前台、停止发声……）后，至少过这么久才会让路：
    /// 来回切换窗口、通知弹窗抢一下前台、声音时断时续，都不会让它反复调整。
    pub reyield_after_ms: Millis,
}

impl BusyRule {
    /// 当前需求下的门槛（处理器、磁盘）；没有需求时为 None（不让路）。
    fn thresholds(&self, pressure: Pressure) -> Option<(f32, f64)> {
        match pressure {
            Pressure::None => None,
            Pressure::Normal => Some((self.core_pct, self.disk_bytes_per_s)),
            Pressure::High => Some((self.core_pct_high, self.disk_bytes_per_s / 2.0)),
        }
    }
}

/// 一个程序此刻的负载。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TreeUsage {
    /// 处理器（单核百分比）。
    pub core_pct: f32,
    /// 磁盘读写（字节每秒）。
    pub disk_bytes_per_s: f64,
}

/// 此刻的需求。
#[derive(Debug, Clone, Copy)]
pub struct Demand {
    pub now: Millis,
    pub pressure: Pressure,
    /// 已经连续多久没有需求（见 `DemandTracker::idle_for`）。
    pub idle_for: Millis,
}

/// 后台忙碌：有需求时，在后台持续占用处理器或磁盘的程序让路；让路后保持，直到它回到"正在用"
/// （前台、发声、游戏等）、退出、长时间没有需求（没人需要资源了，后台的工作应该全速完成），
/// 或者需求降低后它已远低于新的门槛（有重点程序时按更低的门槛让路的程序，重点程序结束后恢复）。
/// 不因为它一时闲下来就恢复，避免反复调整。按进程树（程序）判断。
#[derive(Debug, Default)]
pub struct BusyTracker {
    /// 已经让路的程序，以及按哪一级需求的门槛让的路。
    yielded: HashMap<ProcessKey, Pressure>,
    /// 最近一次"正在用"的时刻（见 [`BusyRule::reyield_after_ms`]）。
    last_used: HashMap<ProcessKey, Millis>,
}

impl BusyTracker {
    /// 需求降低后，负载低于新门槛的这个比例才恢复：在门槛附近的程序不会来回调整。
    const RELEASE_RATIO: f32 = 0.7;

    /// 每轮对每个程序调用一次，返回它现在是否应该让路。
    pub fn observe(
        &mut self,
        root: ProcessKey,
        usage: TreeUsage,
        in_use: bool,
        demand: Demand,
        rule: BusyRule,
    ) -> bool {
        if in_use {
            self.yielded.remove(&root);
            self.last_used.insert(root, demand.now);
            return false;
        }
        if let Some(&tier) = self.yielded.get(&root) {
            return self.keep_yielded(root, tier, usage, demand, rule);
        }
        if let Some(&used) = self.last_used.get(&root) {
            if demand.now.saturating_sub(used) < rule.reyield_after_ms {
                return false;
            }
            self.last_used.remove(&root);
        }
        let Some((core_pct, disk)) = rule.thresholds(demand.pressure) else {
            return false;
        };
        if usage.core_pct >= core_pct || usage.disk_bytes_per_s >= disk {
            self.yielded.insert(root, demand.pressure);
            return true;
        }
        false
    }

    fn keep_yielded(
        &mut self,
        root: ProcessKey,
        tier: Pressure,
        usage: TreeUsage,
        demand: Demand,
        rule: BusyRule,
    ) -> bool {
        if demand.pressure == Pressure::None {
            if demand.idle_for >= rule.release_after_ms {
                self.yielded.remove(&root);
                return false;
            }
            return true;
        }
        if demand.pressure < tier
            && let Some((core_pct, disk)) = rule.thresholds(demand.pressure)
        {
            let ratio = Self::RELEASE_RATIO;
            if usage.core_pct < core_pct * ratio && usage.disk_bytes_per_s < disk * f64::from(ratio)
            {
                self.yielded.remove(&root);
                return false;
            }
            // 在新门槛下依然在忙：继续让路，之后按新门槛判断。
            self.yielded.insert(root, demand.pressure);
        }
        true
    }

    /// 程序不久前还在"正在用"（见 [`BusyRule::reyield_after_ms`]）：先不挪动它。
    pub fn settling(&self, root: &ProcessKey, now: Millis, rule: BusyRule) -> bool {
        self.last_used
            .get(root)
            .is_some_and(|t| now.saturating_sub(*t) < rule.reyield_after_ms)
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.yielded.retain(|k, _| alive.contains(k));
        self.last_used.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: ProcessKey = ProcessKey {
        pid: 10,
        created: 1,
    };

    const RULE: BusyRule = BusyRule {
        core_pct: 10.0,
        core_pct_high: 3.0,
        disk_bytes_per_s: 8.0 * 1024.0 * 1024.0,
        release_after_ms: 60_000,
        reyield_after_ms: 20_000,
    };

    fn cpu(core_pct: f32) -> TreeUsage {
        TreeUsage {
            core_pct,
            disk_bytes_per_s: 0.0,
        }
    }

    fn at(now: Millis, pressure: Pressure) -> Demand {
        Demand {
            now,
            pressure,
            idle_for: 0,
        }
    }

    #[test]
    fn busy_programs_yield_only_when_someone_needs_the_processor() {
        let mut b = BusyTracker::default();
        // 没人需要：再忙也不让路。
        assert!(!b.observe(ROOT, cpu(80.0), false, at(0, Pressure::None), RULE));
        // 有需求：达到门槛才让路，需求高时门槛更低。
        assert!(!b.observe(ROOT, cpu(5.0), false, at(0, Pressure::Normal), RULE));
        assert!(b.observe(ROOT, cpu(5.0), false, at(0, Pressure::High), RULE));
        // 闲下来也保持让路，不来回调整。
        assert!(b.observe(ROOT, cpu(0.0), false, at(0, Pressure::High), RULE));
        // 回到前台（正在用）立即恢复。
        assert!(!b.observe(ROOT, cpu(50.0), true, at(0, Pressure::High), RULE));
    }

    #[test]
    fn yielded_programs_come_back_when_nobody_needs_the_processor() {
        let mut b = BusyTracker::default();
        assert!(b.observe(ROOT, cpu(50.0), false, at(0, Pressure::Normal), RULE));
        let idle = |idle_for| Demand {
            now: 0,
            pressure: Pressure::None,
            idle_for,
        };
        assert!(b.observe(ROOT, cpu(50.0), false, idle(30_000), RULE));
        assert!(!b.observe(ROOT, cpu(50.0), false, idle(60_000), RULE));
    }

    #[test]
    fn programs_just_used_are_not_yielded_again_right_away() {
        let mut b = BusyTracker::default();
        assert!(b.observe(ROOT, cpu(50.0), false, at(0, Pressure::High), RULE));
        // 被点开看了一眼又切走：先不让路。
        assert!(!b.observe(ROOT, cpu(50.0), true, at(2_000, Pressure::High), RULE));
        assert!(!b.observe(ROOT, cpu(50.0), false, at(4_000, Pressure::High), RULE));
        assert!(!b.observe(ROOT, cpu(50.0), false, at(21_000, Pressure::High), RULE));
        // 离开够久了，依然在忙才让路。
        assert!(b.observe(ROOT, cpu(50.0), false, at(23_000, Pressure::High), RULE));
    }

    #[test]
    fn lower_demand_releases_programs_far_below_the_new_threshold() {
        let mut b = BusyTracker::default();
        let light = ProcessKey {
            pid: 11,
            created: 1,
        };
        // 玩游戏时：5% 和 9% 都按 3% 的门槛让路。
        assert!(b.observe(ROOT, cpu(5.0), false, at(0, Pressure::High), RULE));
        assert!(b.observe(light, cpu(9.0), false, at(0, Pressure::High), RULE));
        // 游戏结束，需求回到一般（门槛 10%）：远低于门槛的恢复，接近门槛的继续让路。
        assert!(!b.observe(ROOT, cpu(5.0), false, at(1, Pressure::Normal), RULE));
        assert!(b.observe(light, cpu(9.0), false, at(1, Pressure::Normal), RULE));
    }

    #[test]
    fn heavy_disk_use_counts_as_busy() {
        let mut b = BusyTracker::default();
        let disk = |bytes_per_s| TreeUsage {
            core_pct: 1.0,
            disk_bytes_per_s: bytes_per_s,
        };
        assert!(!b.observe(ROOT, disk(2e6), false, at(0, Pressure::Normal), RULE));
        assert!(b.observe(ROOT, disk(20e6), false, at(0, Pressure::Normal), RULE));
    }
}
