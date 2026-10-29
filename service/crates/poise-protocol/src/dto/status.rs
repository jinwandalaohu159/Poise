//! 状态：主页与托盘面板（健康、模式、读数、硬件信息）。

use serde::{Deserialize, Serialize};

use poise_core::diagnose::Health;
use poise_core::prefs::{GameMode, Mode};

use super::LastCleanup;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol: u32,
    pub service_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub cpu_pct: f32,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub gpu_pct: Option<f32>,
    pub mem_used_pct: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub cpu_mhz: u32,
    pub on_battery: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// 机型档案名，例如 "Dell G15 5520"；没有匹配的档案时为 "Generic"。
    pub profile: String,
    /// 是否能控制散热模式。
    pub thermal_control: bool,
    /// 是否能给风扇加速。
    pub fan_boost_supported: bool,
    /// 机型极限模式的标识（例如 "dell_g_mode"，界面按它显示"G 模式"）；不支持时为空。
    pub vendor_mode: Option<String>,
    /// 风扇转速（只在界面显示或游戏时读取，其余时候为空）。
    pub fans: Vec<FanDto>,
    pub disks: Vec<DiskDto>,
    pub system_drive: Option<DriveSpace>,
    pub memory: Option<MemoryDto>,
    pub cpu: Option<CpuDto>,
}

/// 处理器的核心组成。不是大小核处理器时，全部核心算在"大核"里，小核为 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuDto {
    pub performance_cores: u32,
    pub performance_threads: u32,
    pub efficiency_cores: u32,
    pub efficiency_threads: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanDto {
    pub rpm: u32,
    pub max_rpm: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskHealthDto {
    Healthy,
    Warning,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskDto {
    pub name: String,
    /// 整块硬盘的容量。
    pub size_bytes: Option<u64>,
    pub temp_c: Option<u32>,
    pub health: DiskHealthDto,
    /// 寿命已用百分比。
    pub wear_pct: Option<u32>,
    /// 有盘符的分区，按盘符排序。
    pub volumes: Vec<VolumeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeDto {
    /// 盘符，例如 "C"。
    pub letter: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDto {
    pub total_bytes: u64,
    pub speed_mhz: u32,
    /// 每条内存的容量。
    pub module_bytes: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub health: Health,
    pub mode: Mode,
    pub effective_mode: Mode,
    /// 玩游戏时切换到的模式。
    pub game_mode: GameMode,
    pub auto_clean: bool,
    /// 风扇加速设置，0～100（0 = 不加速）。
    pub fan_boost: u8,
    /// 智能调度开关。
    pub smart_schedule: bool,
    pub metrics: Metrics,
    pub hardware: HardwareInfo,
    /// 当前被调度调整过的进程数（用于"详情"）。
    pub adjusted_processes: usize,
    /// 等待游戏或全屏结束后再处理的清理项。
    pub deferred_cleanup: usize,
    /// 观察模式：只判断、不执行。
    pub dry_run: bool,
    /// 最近一次真正做了事的清理（手动、自动或游戏结束后补做）。
    pub last_cleanup: Option<LastCleanup>,
}
