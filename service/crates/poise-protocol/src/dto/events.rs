//! 日志：Poise 做过的每一件值得知道的事（调度决定、模式切换、限速、清理、电源设置的守护、启动与退出），
//! 结构化记录，界面按语言写成文字、按类别筛选。服务同时把它们写进文本日志 poise.log。

use serde::{Deserialize, Serialize};

use poise_core::model::Pid;
use poise_core::power_settings::PowerSettingKind;
use poise_core::prefs::{AppPreference, Mode};
use poise_core::schedule::{Cause, Pressure, Verdict};

use super::CleanupTrigger;

/// 一次取日志的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPage {
    /// 从早到晚。
    pub entries: Vec<EventEntry>,
    /// 详细日志（poise.log）所在的文件夹。
    pub log_dir: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEntry {
    /// 递增的编号：界面记住最后一条的编号，下次只取更新的。
    pub id: u64,
    pub at_unix_ms: u64,
    pub category: EventCategory,
    pub event: Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    /// 调度：需求、对程序的决定、挪开、恢复。
    Schedule,
    /// 模式与处理器：模式切换、限速。
    Mode,
    /// 电源设置与系统优化：守护、重新应用、用户的修改。
    Power,
    Cleanup,
    /// Poise 自身：启动、开始与停止调度、设置。
    System,
}

/// 涉及的程序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramRef {
    pub pid: Pid,
    pub name: String,
    /// 小写完整路径；读不到时为空。
    pub path: String,
}

/// 程序用到的核心范围（不限制时 `all` 为真）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSpan {
    pub all: bool,
    /// 用主域（大核）。
    pub main: bool,
    /// 用了几个其余的核心域（按缓存分的小核组、其他 CCD）。
    pub others: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// 对处理器的需求变了（决定后台让路到什么程度）。
    Demand {
        level: Pressure,
    },
    /// 对一个程序的决定变了，或重点程序用的核心变了。
    Program {
        program: ProgramRef,
        from: Verdict,
        to: Verdict,
        cores: CoreSpan,
        /// 单核百分比。
        load_pct: u32,
        disk_mb_s: u32,
        /// 没有让路、或者恢复的原因。
        cause: Option<Cause>,
    },
    /// 有重点程序时，没在用的程序移到小核：开始时报告个数，全部移回时为 0。
    MovedAside {
        count: u32,
    },
    /// 系统拒绝调整这个进程（受保护的进程、反作弊），之后不再尝试。
    AccessDenied {
        program: ProgramRef,
    },
    /// 上次异常退出时留下调整过的进程，已把还在运行的恢复原样。
    Recovered {
        left: u32,
        restored: u32,
    },
    /// 模式切换（电源计划与散热已写入）。
    ModeChanged {
        mode: Mode,
        on_battery: bool,
    },
    ModeFailed {
        mode: Mode,
    },
    /// 处理器开始被固件限速（散热或功耗到了上限）；`limit_pct` 是允许的性能上限。
    Throttled {
        limit_pct: Option<f32>,
    },
    Unthrottled,
    /// 电源计划被换了，当前模式已重新写入新计划。
    PlanSwitched,
    /// Poise 写入的电源设置被别处改掉，已改回。
    SettingRestored {
        setting: Option<PowerSettingKind>,
        /// 作用于哪个效率等级（不分等级的为空）。
        class: Option<u8>,
        /// 插电（否则是电池）。
        ac: bool,
        /// 被改成的值（不存在时为空）和 Poise 设的值。
        found: Option<u32>,
        expected: Option<u32>,
    },
    /// 应用过的优化项不再是优化状态，已重新应用。
    OptimizationReapplied {
        id: String,
    },
    /// Windows 电源模式处于"最佳能效"：会盖住电源计划里的部分设置。
    EfficiencySlider,
    /// 用户应用、还原了系统优化项。
    OptimizationsApplied {
        ids: Vec<String>,
    },
    OptimizationsReverted {
        ids: Vec<String>,
    },
    /// 一次真正做了事的清理。
    Cleaned {
        trigger: CleanupTrigger,
        freed_bytes: u64,
        handled: u32,
        failed: u32,
    },
    /// 服务启动。
    Started {
        version: String,
        observe_only: bool,
    },
    /// 托盘代理连上，开始调度。
    Resumed,
    /// 托盘代理退出：进程调整、散热与电源计划已全部还原。
    Paused,
    /// 用户设置了程序的类别。
    PreferenceSet {
        name: String,
        path: String,
        preference: AppPreference,
    },
    /// 用户打开、关闭了智能调度。
    SmartSchedule {
        enabled: bool,
    },
}

impl Event {
    pub fn category(&self) -> EventCategory {
        match self {
            Self::Demand { .. }
            | Self::Program { .. }
            | Self::MovedAside { .. }
            | Self::AccessDenied { .. }
            | Self::Recovered { .. } => EventCategory::Schedule,
            Self::ModeChanged { .. }
            | Self::ModeFailed { .. }
            | Self::Throttled { .. }
            | Self::Unthrottled => EventCategory::Mode,
            Self::PlanSwitched
            | Self::SettingRestored { .. }
            | Self::OptimizationReapplied { .. }
            | Self::EfficiencySlider
            | Self::OptimizationsApplied { .. }
            | Self::OptimizationsReverted { .. } => EventCategory::Power,
            Self::Cleaned { .. } => EventCategory::Cleanup,
            Self::Started { .. }
            | Self::Resumed
            | Self::Paused
            | Self::PreferenceSet { .. }
            | Self::SmartSchedule { .. } => EventCategory::System,
        }
    }
}
