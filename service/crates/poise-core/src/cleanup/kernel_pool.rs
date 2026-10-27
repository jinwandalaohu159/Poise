//! 内核内存池（非分页池）：驱动泄漏时只涨不跌，只有真重启能回收。
//!
//! 只在"偏大且仍在上涨"或"大到明显异常"时建议重启；稳定在偏高水平的不打扰。
//! 用户选择"稍后提醒"后，除非又明显上涨，否则不再提示。

use crate::model::Millis;

use super::trend::{GrowthRule, sustained_growth};

const MB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy)]
pub struct KernelPoolConfig {
    /// 两个保存点之间的最小间隔（降采样）。
    pub sample_interval_ms: Millis,
    /// 最多保存多少个点（默认 24 小时）。
    pub max_points: usize,
    /// 低于这个值时，即使在上涨也不提示。
    pub min_bytes: u64,
    /// 超过这个值时，不看趋势也提示。
    pub severe_bytes: u64,
    pub growth: GrowthRule,
    /// 稍后提醒之后，再涨这么多才重新提示。
    pub snooze_regrowth_bytes: u64,
}

impl Default for KernelPoolConfig {
    fn default() -> Self {
        Self {
            sample_interval_ms: 5 * 60_000,
            max_points: 288,
            min_bytes: 1024 * 1024 * 1024,
            severe_bytes: 2 * 1024 * 1024 * 1024,
            growth: GrowthRule {
                min_span_ms: 60 * 60_000,
                min_points: 8,
                min_r2: 0.6,
                min_slope_per_hour: 20.0 * MB,
                min_growth: 64.0 * MB,
                min_growth_ratio: 0.05,
            },
            snooze_regrowth_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default)]
pub struct KernelPoolWatch {
    cfg: KernelPoolConfig,
    points: Vec<(Millis, u64)>,
    latest: u64,
    /// 用户选择"稍后提醒"时的大小。
    snoozed_at: Option<u64>,
}

impl KernelPoolWatch {
    pub fn new(cfg: KernelPoolConfig) -> Self {
        Self {
            cfg,
            ..Default::default()
        }
    }

    pub fn observe(&mut self, now: Millis, nonpaged: u64) {
        self.latest = nonpaged;
        let due = self
            .points
            .last()
            .is_none_or(|(at, _)| now.saturating_sub(*at) >= self.cfg.sample_interval_ms);
        if due {
            self.points.push((now, nonpaged));
            if self.points.len() > self.cfg.max_points {
                self.points.remove(0);
            }
        }
    }

    /// 是否建议真重启。
    pub fn restart_advised(&self) -> bool {
        let current = self.latest;
        if current < self.cfg.min_bytes {
            return false;
        }
        if self
            .snoozed_at
            .is_some_and(|s| current < s.saturating_add(self.cfg.snooze_regrowth_bytes))
        {
            return false;
        }
        current >= self.cfg.severe_bytes
            || sustained_growth(&self.points, |p| p.0, |p| p.1 as f64, &self.cfg.growth).is_some()
    }

    /// 用户选择"稍后提醒"。
    pub fn snooze(&mut self) {
        self.snoozed_at = Some(self.latest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Millis = 60_000;
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;

    fn feed(w: &mut KernelPoolWatch, f: impl Fn(u64) -> u64) {
        for i in 0..=24 {
            w.observe(i * 5 * MIN, f(i));
        }
    }

    #[test]
    fn small_or_stable_pool_is_quiet() {
        let mut w = KernelPoolWatch::default();
        feed(&mut w, |i| 300 * MB + i * 10 * MB);
        assert!(!w.restart_advised());
        let mut w = KernelPoolWatch::default();
        feed(&mut w, |_| GB + 500 * MB);
        assert!(!w.restart_advised());
    }

    #[test]
    fn large_and_rising_pool_advises_restart() {
        let mut w = KernelPoolWatch::default();
        feed(&mut w, |i| GB + i * 10 * MB);
        assert!(w.restart_advised());
    }

    #[test]
    fn severe_pool_advises_restart_without_history() {
        let mut w = KernelPoolWatch::default();
        w.observe(0, 2 * GB + MB);
        assert!(w.restart_advised());
    }

    #[test]
    fn snooze_holds_until_pool_grows_again() {
        let mut w = KernelPoolWatch::default();
        w.observe(0, 2 * GB + MB);
        w.snooze();
        w.observe(MIN, 2 * GB + 100 * MB);
        assert!(!w.restart_advised());
        w.observe(2 * MIN, 2 * GB + 300 * MB);
        assert!(w.restart_advised());
    }
}
