//! 整机：本次开机的基线，以及现在与它的逐项对比。

use serde::{Deserialize, Serialize};

use super::GrowthLimit;
use crate::model::SystemMetrics;

/// 整机总量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemTotals {
    pub handles: u32,
    pub threads: u32,
    pub processes: u32,
    pub commit_bytes: u64,
    pub kernel_paged_bytes: u64,
    pub kernel_nonpaged_bytes: u64,
    pub system_cache_bytes: u64,
}

impl From<&SystemMetrics> for SystemTotals {
    fn from(m: &SystemMetrics) -> Self {
        Self {
            handles: m.handle_count,
            threads: m.thread_count,
            processes: m.process_count,
            commit_bytes: m.commit_total,
            kernel_paged_bytes: m.kernel_paged,
            kernel_nonpaged_bytes: m.kernel_nonpaged,
            system_cache_bytes: m.system_cache,
        }
    }
}

/// 本次开机的整机基线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootBaseline {
    /// 开机时刻（Unix 秒），用来判断保存的基线是不是本次开机的。
    pub boot_unix_s: u64,
    /// 记录基线时距开机多久。
    pub uptime_ms: u64,
    pub totals: SystemTotals,
}

impl BootBaseline {
    /// 开机时刻允许的误差（由"现在 - 开机时长"推算，会有几秒抖动）。
    const BOOT_TOLERANCE_S: u64 = 60;

    pub fn is_same_boot(&self, boot_unix_s: u64) -> bool {
        self.boot_unix_s.abs_diff(boot_unix_s) <= Self::BOOT_TOLERANCE_S
    }
}

/// 本次开机的整机基线：系统稳定下来时记录一次；沿用上次保存的，只要属于本次开机。
#[derive(Debug, Default)]
pub(super) struct BootWatch {
    baseline: Option<BootBaseline>,
}

impl BootWatch {
    pub(super) fn new(restored: Option<BootBaseline>, boot_unix_s: u64) -> Self {
        Self {
            baseline: restored.filter(|b| b.is_same_boot(boot_unix_s)),
        }
    }

    /// 还没有本次开机的基线、并且开机已经超过 `settle_ms` 时记录，返回新记录的基线。
    pub(super) fn observe(
        &mut self,
        system: &SystemMetrics,
        boot_unix_s: u64,
        settle_ms: u64,
    ) -> Option<BootBaseline> {
        let current = self.baseline.is_some_and(|b| b.is_same_boot(boot_unix_s));
        if current || system.uptime_ms < settle_ms {
            return None;
        }
        let baseline = BootBaseline {
            boot_unix_s,
            uptime_ms: system.uptime_ms,
            totals: SystemTotals::from(system),
        };
        self.baseline = Some(baseline);
        Some(baseline)
    }

    pub(super) fn baseline(&self) -> Option<&BootBaseline> {
        self.baseline.as_ref()
    }
}

/// 整机的一项总量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TotalItem {
    Handles,
    Threads,
    Processes,
    Commit,
    /// 内核内存（分页 + 非分页池）。
    KernelPool,
    /// 系统文件缓存：只展示，不判断、不清理（它会在需要时自动让出）。
    SystemCache,
}

/// 与开机时相比的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotalChange {
    pub item: TotalItem,
    pub at_boot: u64,
    pub now: u64,
    /// 增长明显超出正常使用的范围。
    pub notable: bool,
}

/// 整机总量与开机基线逐项对比。
pub fn compare_with_boot(boot: &SystemTotals, now: &SystemTotals) -> Vec<TotalChange> {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    let limit = |min_increase, min_ratio| {
        Some(GrowthLimit {
            min_increase,
            min_ratio,
        })
    };
    [
        (
            TotalItem::Handles,
            u64::from(boot.handles),
            u64::from(now.handles),
            limit(40_000, 1.5),
        ),
        (
            TotalItem::Threads,
            u64::from(boot.threads),
            u64::from(now.threads),
            limit(2_000, 1.5),
        ),
        (
            TotalItem::Processes,
            u64::from(boot.processes),
            u64::from(now.processes),
            limit(100, 1.5),
        ),
        (
            TotalItem::Commit,
            boot.commit_bytes,
            now.commit_bytes,
            limit(8 * GB, 2.0),
        ),
        (
            TotalItem::KernelPool,
            boot.kernel_paged_bytes + boot.kernel_nonpaged_bytes,
            now.kernel_paged_bytes + now.kernel_nonpaged_bytes,
            limit(512 * MB, 2.0),
        ),
        (
            TotalItem::SystemCache,
            boot.system_cache_bytes,
            now.system_cache_bytes,
            None,
        ),
    ]
    .into_iter()
    .map(|(item, at_boot, now, limit)| TotalChange {
        item,
        at_boot,
        now,
        notable: limit.is_some_and(|l| l.exceeded(at_boot, now)),
    })
    .collect()
}
