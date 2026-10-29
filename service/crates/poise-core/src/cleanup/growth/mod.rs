//! 与"开机时""进程刚启动时"相比的增长。
//!
//! - 整机（`system`）：开机后系统稳定下来时记一次基线（句柄、线程、进程数、提交内存、内核内存、系统缓存），
//!   之后随时可以和它比较。基线按本次开机保存，服务重启不丢失。
//! - 进程（`process`）：每个进程运行稳定后记一次自己的基线（句柄、线程、私有内存）。在 Poise 启动前就在运行的进程，
//!   从 Poise 第一次看到它时算起。
//! - "平白无故变多"：观察足够久之后，增量与倍数**同时**超过阈值才算，避免把正常使用误判成问题。

mod process;
mod system;

pub use process::{Change, Grown, GrowthMetric};
pub use system::{BootBaseline, SystemTotals, TotalChange, TotalItem, compare_with_boot};

use crate::model::{Millis, Snapshot};

use process::ProcessGrowth;
use system::BootWatch;

/// 增量至少 `min_increase`，并且至少是基线的 `min_ratio` 倍。
#[derive(Debug, Clone, Copy)]
pub struct GrowthLimit {
    pub min_increase: u64,
    pub min_ratio: f64,
}

impl GrowthLimit {
    pub(super) fn exceeded(&self, base: u64, now: u64) -> bool {
        now >= base.saturating_add(self.min_increase)
            && now as f64 >= base.max(1) as f64 * self.min_ratio
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GrowthConfig {
    /// 开机后多久记录整机基线（等开机自启的程序都起来）。
    pub boot_settle_ms: Millis,
    /// 进程被看到后多久记录它的基线（等它初始化完）。
    pub start_settle_ms: Millis,
    /// 记录基线后至少观察这么久才判断异常增长。
    pub min_observed_ms: Millis,
    pub handles: GrowthLimit,
    pub threads: GrowthLimit,
    pub memory: GrowthLimit,
}

impl Default for GrowthConfig {
    fn default() -> Self {
        Self {
            boot_settle_ms: 5 * 60_000,
            start_settle_ms: 2 * 60_000,
            min_observed_ms: 30 * 60_000,
            handles: GrowthLimit {
                min_increase: 10_000,
                min_ratio: 3.0,
            },
            threads: GrowthLimit {
                min_increase: 500,
                min_ratio: 3.0,
            },
            memory: GrowthLimit {
                min_increase: 1024 * 1024 * 1024,
                min_ratio: 3.0,
            },
        }
    }
}

/// 整机与每个进程的增长观察。
#[derive(Debug, Default)]
pub struct GrowthWatch {
    cfg: GrowthConfig,
    boot: BootWatch,
    processes: ProcessGrowth,
}

impl GrowthWatch {
    /// `restored`：上次保存的整机基线，只有属于本次开机时才沿用。
    pub fn new(cfg: GrowthConfig, restored: Option<BootBaseline>, boot_unix_s: u64) -> Self {
        Self {
            cfg,
            boot: BootWatch::new(restored, boot_unix_s),
            processes: ProcessGrowth::default(),
        }
    }

    /// 每次采样后调用。新记录了整机基线时返回它（由调用方保存）。
    pub fn observe(
        &mut self,
        snap: &Snapshot,
        now: Millis,
        boot_unix_s: u64,
    ) -> Option<BootBaseline> {
        self.processes.observe(snap, now, self.cfg.start_settle_ms);
        self.boot
            .observe(&snap.system, boot_unix_s, self.cfg.boot_settle_ms)
    }

    pub fn boot_baseline(&self) -> Option<&BootBaseline> {
        self.boot.baseline()
    }

    /// 观察足够久、并且明显超出自身基线的进程（每个进程只报最突出的一项：句柄 > 线程 > 内存）。
    pub fn grown(&self, snap: &Snapshot, now: Millis) -> Vec<Grown> {
        self.processes.grown(snap, now, &self.cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessKey, ProcessMetrics, ProcessSample, SystemMetrics};

    const MIN: Millis = 60_000;

    fn snap(uptime_ms: u64, procs: &[(u32, u32, u32)]) -> Snapshot {
        Snapshot {
            processes: procs
                .iter()
                .map(|&(pid, handles, threads)| ProcessSample {
                    facts: ProcessFacts {
                        key: ProcessKey { pid, created: 1 },
                        ..Default::default()
                    },
                    metrics: ProcessMetrics {
                        handles,
                        threads,
                        private_bytes: 100 << 20,
                        ..Default::default()
                    },
                })
                .collect(),
            system: SystemMetrics {
                uptime_ms,
                handle_count: 80_000,
                thread_count: 3_000,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn boot_baseline_waits_for_the_system_to_settle_and_is_recorded_once() {
        let mut w = GrowthWatch::new(GrowthConfig::default(), None, 1_000);
        assert!(w.observe(&snap(2 * MIN, &[]), 0, 1_000).is_none());
        let b = w.observe(&snap(6 * MIN, &[]), 4 * MIN, 1_000).unwrap();
        assert_eq!((b.totals.handles, b.uptime_ms), (80_000, 6 * MIN));
        assert!(w.observe(&snap(7 * MIN, &[]), 5 * MIN, 1_000).is_none());
    }

    #[test]
    fn a_saved_baseline_is_kept_only_for_the_same_boot() {
        let saved = BootBaseline {
            boot_unix_s: 1_000,
            uptime_ms: 5 * MIN,
            totals: SystemTotals::default(),
        };
        assert!(
            GrowthWatch::new(GrowthConfig::default(), Some(saved), 1_030)
                .boot_baseline()
                .is_some()
        );
        assert!(
            GrowthWatch::new(GrowthConfig::default(), Some(saved), 9_000)
                .boot_baseline()
                .is_none()
        );
    }

    #[test]
    fn handles_that_keep_piling_up_are_reported_after_enough_time() {
        let mut w = GrowthWatch::new(GrowthConfig::default(), None, 0);
        w.observe(&snap(10 * MIN, &[(1, 2_000, 40)]), 0, 0);
        w.observe(&snap(13 * MIN, &[(1, 2_000, 40)]), 3 * MIN, 0); // 记录基线：2000 句柄
        // 观察时间不够：不报。
        let early = snap(20 * MIN, &[(1, 30_000, 40)]);
        assert!(w.grown(&early, 10 * MIN).is_empty());
        // 40 分钟后：30000 ≥ 2000 + 10000，且 ≥ 3 倍。
        let later = snap(50 * MIN, &[(1, 30_000, 40)]);
        assert_eq!(
            w.grown(&later, 40 * MIN),
            vec![Grown {
                key: ProcessKey { pid: 1, created: 1 },
                change: Change {
                    metric: GrowthMetric::Handles,
                    from: 2_000,
                    to: 30_000,
                },
            }]
        );
    }

    #[test]
    fn large_but_proportionate_growth_is_normal() {
        let mut w = GrowthWatch::new(GrowthConfig::default(), None, 0);
        w.observe(&snap(10 * MIN, &[(1, 20_000, 400)]), 0, 0);
        w.observe(&snap(13 * MIN, &[(1, 20_000, 400)]), 3 * MIN, 0);
        // 句柄 +15000 但只有 1.75 倍；线程 +600 但只有 2.5 倍：都不算。
        let later = snap(60 * MIN, &[(1, 35_000, 1_000)]);
        assert!(w.grown(&later, 60 * MIN).is_empty());
    }

    #[test]
    fn exited_processes_are_forgotten() {
        let mut w = GrowthWatch::new(GrowthConfig::default(), None, 0);
        w.observe(&snap(10 * MIN, &[(1, 100, 4)]), 0, 0);
        w.observe(&snap(10 * MIN, &[]), MIN, 0);
        assert!(w.processes.is_empty());
    }

    #[test]
    fn system_comparison_flags_only_disproportionate_growth() {
        let boot = SystemTotals {
            handles: 80_000,
            threads: 3_000,
            kernel_nonpaged_bytes: 300 << 20,
            system_cache_bytes: 1 << 30,
            ..Default::default()
        };
        let now = SystemTotals {
            handles: 160_000,
            threads: 3_500,
            kernel_nonpaged_bytes: 1_200 << 20,
            system_cache_bytes: 8 << 30,
            ..Default::default()
        };
        let notable: Vec<TotalItem> = compare_with_boot(&boot, &now)
            .into_iter()
            .filter(|c| c.notable)
            .map(|c| c.item)
            .collect();
        // 系统缓存涨得再多也不算问题。
        assert_eq!(notable, vec![TotalItem::Handles, TotalItem::KernelPool]);
    }
}
