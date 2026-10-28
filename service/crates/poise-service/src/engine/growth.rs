//! 与开机时、与进程稳定运行时相比的增长：观测、按开机保存整机基线、给清理页的对比。

use poise_core::cleanup::{SystemTotals, compare_with_boot};
use poise_core::model::{Millis, Snapshot};
use poise_protocol::SinceBoot;

use super::{Engine, unix_now_ms};

/// 整机基线保存在数据目录的这个文件里（服务重启后，同一次开机内沿用）。
pub(super) const BASELINE_FILE: &str = "boot_baseline.json";

impl Engine {
    /// 每个采样周期调用：更新进程基线；系统稳定后记录整机基线并保存。
    pub(super) fn observe_growth(&mut self, snapshot: &Snapshot, now: Millis) {
        let boot = boot_unix_s(snapshot.system.uptime_ms);
        if let Some(baseline) = self.growth.observe(snapshot, now, boot) {
            log::info!(
                "已记录开机基线：句柄 {}，线程 {}，进程 {}",
                baseline.totals.handles,
                baseline.totals.threads,
                baseline.totals.processes
            );
            if let Err(e) = self.baseline_file.save(&baseline) {
                log::warn!("保存开机基线失败：{e}");
            }
        }
    }

    /// 清理页"与开机时相比"。
    pub(super) fn since_boot(&self, snapshot: &Snapshot) -> Option<SinceBoot> {
        let baseline = self.growth.boot_baseline()?;
        Some(SinceBoot {
            baseline_uptime_ms: baseline.uptime_ms,
            baseline_at_unix_ms: baseline.boot_unix_s * 1000 + baseline.uptime_ms,
            uptime_ms: snapshot.system.uptime_ms,
            items: compare_with_boot(&baseline.totals, &SystemTotals::from(&snapshot.system)),
        })
    }
}

/// 本次开机的时刻（Unix 秒），由"现在 - 开机时长"推算。
pub(super) fn boot_unix_s(uptime_ms: u64) -> u64 {
    (unix_now_ms() / 1000).saturating_sub(uptime_ms / 1000)
}
