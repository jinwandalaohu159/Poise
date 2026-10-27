//! 返回给界面的数据结构。界面只负责显示，所有判断都在服务里完成。

use serde::{Deserialize, Serialize};

use poise_core::diagnose::Health;
use poise_core::model::Pid;
use poise_core::prefs::{AppPreference, Mode};

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
    /// 风扇转速（只在界面显示或游戏时读取，其余时候为空）。
    pub fans: Vec<FanDto>,
    pub disks: Vec<DiskDto>,
    pub system_drive: Option<DriveSpace>,
    pub memory: Option<MemoryDto>,
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
    pub temp_c: Option<u32>,
    pub health: DiskHealthDto,
    /// 寿命已用百分比。
    pub wear_pct: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDto {
    pub total_bytes: u64,
    pub speed_mhz: u32,
    pub modules: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub health: Health,
    pub mode: Mode,
    pub effective_mode: Mode,
    pub auto_full_for_games: bool,
    pub auto_clean: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupTrigger {
    Manual,
    Auto,
    /// 用户点了清理，当时在玩游戏，游戏结束后补做的。
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
    HeavyApp { name: String, private_bytes: u64 },
    RestartRecommended,
    ManualPending { count: usize },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub freed_bytes: u64,
    /// 已处理的项目（友好名称）。
    pub handled: Vec<String>,
    pub failed: Vec<String>,
    /// 因为正在玩游戏，留到之后处理的项目数。
    pub deferred: usize,
    pub advisories: Vec<AdvisoryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    pub pid: Option<Pid>,
    pub name: String,
    /// 小写完整路径，界面用它取图标、设置偏好。
    pub path: String,
    pub preference: AppPreference,
    pub running: bool,
    pub is_game: bool,
}
