//! 程序偏好页：程序列表、每个程序和进程的类别与调度状态。

use serde::{Deserialize, Serialize};

use poise_core::model::Pid;
use poise_core::prefs::AppPreference;

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
    /// 在哪些核心上运行；不是大小核处理器时为空。
    pub cores: Option<CoreUse>,
    pub priority: PriorityLevel,
    /// Poise 对它做了什么。
    pub adjustment: Adjustment,
}

/// 进程在大小核处理器上的核心范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreUse {
    /// 不限制。
    Any,
    /// 固定在大核（正在玩的游戏）。
    Performance,
    /// 让到小核。
    Efficiency,
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
