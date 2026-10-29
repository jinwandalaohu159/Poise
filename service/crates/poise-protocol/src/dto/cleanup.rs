//! 清理页：一键清理的结果、清理概览、与开机时相比。

use serde::{Deserialize, Serialize};

use poise_core::cleanup::{ProcessGroup, TotalChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupTrigger {
    Manual,
    Auto,
    /// 用户点了清理，当时前台有游戏或全屏程序，退出后补做的。
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCleanup {
    /// Unix 时间（毫秒）。
    pub at_unix_ms: u64,
    pub trigger: CleanupTrigger,
    pub freed_bytes: u64,
    pub handled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdvisoryDto {
    HeavyApp { name: String, memory_bytes: u64 },
    RestartRecommended,
    ManualPending { count: usize },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub freed_bytes: u64,
    /// 已处理的项目（友好名称）。
    pub handled: Vec<String>,
    pub failed: Vec<String>,
    /// 因为前台有游戏或全屏程序，留到之后处理的项目数。
    pub deferred: usize,
    pub advisories: Vec<AdvisoryDto>,
}

/// 清理页的全部数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanupOverview {
    /// 一键清理预计释放的内存。
    pub freeable_bytes: u64,
    /// 需要处理的项目数（含只提示的项目）。
    pub needs_attention: usize,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    /// 内核内存持续增长，需要重启电脑。
    pub restart_advised: bool,
    pub auto_clean: bool,
    pub last_cleanup: Option<LastCleanup>,
    /// 与开机时相比；开机后系统还没稳定（约 5 分钟）时为空。
    pub since_boot: Option<SinceBoot>,
    /// 需要处理的在前，其余按内存占用从高到低。
    pub processes: Vec<ProcessGroup>,
}

/// 整机总量与开机基线的对比。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SinceBoot {
    /// 记录基线时距开机多久。
    pub baseline_uptime_ms: u64,
    /// 记录基线的时刻（Unix 毫秒）。Poise 在开机很久后才首次运行时，基线是那时记录的。
    pub baseline_at_unix_ms: u64,
    /// 现在距开机多久。
    pub uptime_ms: u64,
    pub items: Vec<TotalChange>,
}
