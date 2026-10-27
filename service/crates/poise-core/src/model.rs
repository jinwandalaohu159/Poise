//! 进程与系统的观测数据模型。由平台层采集，业务逻辑只读不写。

use serde::{Deserialize, Serialize};

pub type Pid = u32;

/// 毫秒时间戳（单调时钟或 Unix 时间均可，同一次运行内保持一致即可）。
pub type Millis = u64;

/// 进程的稳定身份。PID 会被系统复用，所以与创建时间一起作为键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ProcessKey {
    pub pid: Pid,
    /// 进程创建时间（FILETIME，100ns）。
    pub created: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Protection {
    #[default]
    None,
    /// PPL：杀软、反作弊、部分系统组件。
    Light,
    /// 完全受保护进程。
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl PriorityClass {
    pub fn rank(self) -> i8 {
        match self {
            Self::Idle => -2,
            Self::BelowNormal => -1,
            Self::Normal => 0,
            Self::AboveNormal => 1,
            Self::High => 2,
            Self::Realtime => 3,
        }
    }
}

/// 某个进程里承载的一个 Windows 服务。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceFacts {
    pub name: String,
    pub accepts_stop: bool,
    /// 依赖它且正在运行的服务数量。
    pub running_dependents: u32,
}

/// 进程的静态属性（生命周期内基本不变）。
#[derive(Debug, Clone, Default)]
pub struct ProcessFacts {
    pub key: ProcessKey,
    pub parent_pid: Option<Pid>,
    /// 小写的可执行文件名，例如 `audiodg.exe`。
    pub name: String,
    /// 小写的完整路径；无权读取时为 None。
    pub image_path: Option<String>,
    /// 程序的友好名称（文件描述），用于界面展示。
    pub display_name: Option<String>,
    pub session_id: u32,
    pub is_critical: bool,
    pub protection: Protection,
    /// 当前优先级；无权读取时为 None。
    pub priority: Option<PriorityClass>,
    pub services: Vec<ServiceFacts>,
}

impl ProcessFacts {
    pub fn pid(&self) -> Pid {
        self.key.pid
    }

    /// 用于界面展示的名称：优先友好名称，否则去掉 .exe 的文件名。
    pub fn friendly_name(&self) -> String {
        match &self.display_name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => self.name.trim_end_matches(".exe").to_string(),
        }
    }

    pub fn is_in_windows_dir(&self) -> bool {
        self.image_path
            .as_deref()
            .is_some_and(|p| p.starts_with(r"c:\windows\"))
    }
}

/// 进程的动态指标（每次采样都会变化）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessMetrics {
    pub private_bytes: u64,
    pub working_set: u64,
    pub handles: u32,
    pub threads: u32,
    /// 累计 CPU 时间（内核 + 用户，100ns）。
    pub cpu_time: u64,
    /// 累计 IO 字节数（读 + 写 + 其他）。
    pub io_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessSample {
    pub facts: ProcessFacts,
    pub metrics: ProcessMetrics,
}

/// 系统级指标。CPU 时间为累计值，由 [`crate::activity::ActivityTracker`] 换算成百分比。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SystemMetrics {
    pub logical_cpus: u32,
    pub cpu_busy_time: u64,
    pub cpu_total_time: u64,
    pub mem_total: u64,
    pub mem_available: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub kernel_nonpaged: u64,
    pub kernel_paged: u64,
    pub cpu_mhz_current: u32,
    pub cpu_mhz_max: u32,
    pub uptime_ms: u64,
}

impl SystemMetrics {
    pub fn mem_used_pct(&self) -> f32 {
        if self.mem_total == 0 {
            return 0.0;
        }
        (self.mem_total - self.mem_available.min(self.mem_total)) as f32 * 100.0
            / self.mem_total as f32
    }

    pub fn commit_pct(&self) -> f32 {
        if self.commit_limit == 0 {
            return 0.0;
        }
        self.commit_total as f32 * 100.0 / self.commit_limit as f32
    }
}

/// 一次完整的采样。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub at: Millis,
    pub processes: Vec<ProcessSample>,
    pub system: SystemMetrics,
}

impl Snapshot {
    pub fn find(&self, pid: Pid) -> Option<&ProcessSample> {
        self.processes.iter().find(|p| p.facts.key.pid == pid)
    }
}

/// 用户会话里的状态，由界面代理上报（服务在会话 0，看不到用户桌面）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: u32,
    pub foreground_pid: Option<Pid>,
    /// 前台是否为全屏程序（游戏、全屏视频、演示）。
    pub fullscreen: bool,
    pub visible_window_pids: Vec<Pid>,
    pub audio_pids: Vec<Pid>,
    /// 桌面外壳（资源管理器）的 PID。
    pub shell_pid: Option<Pid>,
    pub idle_ms: u64,
}
