//! 返回给界面的数据结构。界面只负责显示，所有判断都在服务里完成。

use serde::{Deserialize, Serialize};

use poise_core::cleanup::{ProcessGroup, TotalChange};
use poise_core::diagnose::{Health, HistoryPoint, StutterReport};
use poise_core::model::Pid;
use poise_core::prefs::{AppPreference, GameMode, Mode};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppEntry {
    pub pid: Option<Pid>,
    pub name: String,
    /// 小写完整路径，界面用它取图标、设置偏好。
    pub path: String,
    pub preference: AppPreference,
    pub running: bool,
    pub is_game: bool,
    /// 当前的调度状态（主进程）；没有运行时为空。
    pub schedule: Option<AppSchedule>,
    pub kind: AppKind,
    /// 全部进程合计占整机 CPU 的百分比；没有运行时为 0。
    pub cpu_pct: f32,
    /// 全部进程合计的专用工作集（任务管理器"内存"列）；没有运行时为 0。
    pub memory_bytes: u64,
    /// 组成这个程序的进程，主进程在前；没有运行时为空。
    pub processes: Vec<AppProcess>,
}

/// 程序名下的一个进程（程序偏好页展开后显示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppProcess {
    pub pid: Pid,
    /// 进程文件名。
    pub name: String,
    /// 小写完整路径（读不到时为空）。
    pub path: String,
    /// 可以单独设置类别时为它当前的类别（同一程序里有自己窗口的另一个程序，例如客户端里的游戏本体）；
    /// 否则为空，跟随所属程序。
    pub preference: Option<AppPreference>,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    /// 这个进程自己的调度状态（同一程序里个别进程可能不参与调度或自行管理）。
    pub schedule: AppSchedule,
}

/// 程序偏好页按它分栏：用户程序可以选择类别，系统一栏只供查看。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// 用户程序：按类别调度，可以修改类别。
    App,
    /// 系统与驱动相关的程序（输入法、显卡与声卡助手、安全软件、WSL 等）：不参与调度。
    Excluded,
}

/// 一个程序当前的调度状态（程序偏好页显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSchedule {
    pub state: ScheduleState,
    pub foreground: bool,
    /// 是否限制在小核上运行；不是大小核处理器时为空。
    pub small_cores: Option<bool>,
    pub priority: PriorityLevel,
    /// Poise 对它做了什么。
    pub adjustment: Adjustment,
}

/// Poise 对一个进程做的调整。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adjustment {
    /// 没有调整（优先级等都是程序或系统自己的）。
    #[default]
    None,
    /// 让路：挪到小核、降低优先级、节能。
    Yielded,
    /// 不让系统自动节能限速（正在玩、正在发声、始终优先）。
    Protected,
    /// 标记为游戏：优先级提高一级，并且不让系统自动节能限速。
    Raised,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleState {
    /// 参与调度（是否调整了见 [`AppSchedule::adjustment`]）。
    Scheduled,
    /// 不参与调度：受保护、系统组件、输入/显示/声音/安全相关，或系统拒绝访问。
    Excluded,
    /// 程序自行管理优先级，Poise 已经退让。
    SelfManaged,
    /// 智能调度已关闭。
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityLevel {
    Lower,
    Normal,
    Higher,
    High,
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

/// 卡顿诊断：结论与依据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    #[serde(flatten)]
    pub report: StutterReport,
    /// 诊断时智能调度是否开启（便于开关前后对比）。
    pub smart_schedule: bool,
}

/// 最近几分钟的走势（主页与卡顿诊断共用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trend {
    /// 横轴覆盖的秒数。
    pub window_secs: u32,
    /// 从早到晚；比横轴多几秒，走势图左端不留空。
    pub points: Vec<HistoryPoint>,
}
