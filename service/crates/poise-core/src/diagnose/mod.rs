//! 诊断：把指标归结成结论：当前状态（health）和卡顿原因（stutter）。

mod health;
mod heat;
mod stutter;

pub use health::{Health, HealthCode, HealthInput, Level, assess};
pub use heat::{Heat, HeatWatch};
pub use stutter::{StutterCause, StutterInput, StutterReport, analyze_stutter};

use serde::{Deserialize, Serialize};

use crate::model::SystemMetrics;

/// 温度读数（读不到的为 None）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Thermals {
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnoseConfig {
    /// 平滑后的温度达到这个值进入"温度过高"。
    pub hot_temp: f32,
    /// 已经"温度过高"时，降到这个值以下才退出（避免在阈值附近反复切换）。
    pub hot_release_temp: f32,
    pub critical_temp: f32,
    /// 温度平滑的时间常数：短于它的尖峰影响很小。
    pub heat_smoothing_ms: u64,
    pub commit_tired_pct: f32,
    /// 负载高于这个百分比时才判断是否降频。
    pub throttle_load_pct: f32,
    /// 频率低于基础频率的这个比例视为降频。
    pub throttle_ratio: f32,
    /// 系统盘剩余低于这个比例，或低于下面的绝对值，视为快满了。
    pub storage_low_ratio: f64,
    pub storage_low_bytes: u64,
}

impl Default for DiagnoseConfig {
    fn default() -> Self {
        Self {
            hot_temp: 90.0,
            hot_release_temp: 85.0,
            critical_temp: 95.0,
            heat_smoothing_ms: 20_000,
            commit_tired_pct: 85.0,
            throttle_load_pct: 60.0,
            throttle_ratio: 0.85,
            storage_low_ratio: 0.10,
            storage_low_bytes: 15 * 1024 * 1024 * 1024,
        }
    }
}

/// 系统盘剩余空间是否太少。
pub fn is_storage_low(cfg: &DiagnoseConfig, free: u64, total: u64) -> bool {
    total > 0
        && (free < cfg.storage_low_bytes || (free as f64) < total as f64 * cfg.storage_low_ratio)
}

/// 高负载下频率掉到基础频率以下，说明散热或功耗墙在限制性能。
pub fn is_throttling(cfg: &DiagnoseConfig, system: &SystemMetrics, cpu_pct: f32) -> bool {
    system.cpu_mhz_max > 0
        && cpu_pct >= cfg.throttle_load_pct
        && (system.cpu_mhz_current as f32) < system.cpu_mhz_max as f32 * cfg.throttle_ratio
}
