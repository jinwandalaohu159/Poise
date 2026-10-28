//! 所有会改变系统运行状态的动作。动作只能交给 [`crate::safety::GuardedExecutor`] 执行。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{Pid, PriorityClass};

/// 效率模式（EcoQoS）状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EcoQos {
    /// 交给系统决定（不干预）。
    #[default]
    Default,
    /// 明确关闭节能限制：不允许系统对这个进程降频、放宽计时器精度（Windows 会对后台、
    /// 最小化、被遮挡的程序自动这样做，游戏切出去、后台播放声音时会因此卡顿）。
    Off,
    /// 开启效率模式：优先放到 E 核并降频。
    On,
}

/// 进程可用的核心集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CoreSet {
    /// 不限制。
    #[default]
    All,
    Performance,
    Efficiency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    SetPriority {
        pid: Pid,
        class: PriorityClass,
    },
    SetEcoQos {
        pid: Pid,
        state: EcoQos,
    },
    SetCoreSet {
        pid: Pid,
        set: CoreSet,
    },
    /// 重启承载或拉起该进程的服务。
    RestartService {
        pid: Pid,
        service: String,
    },
    /// 结束合成器进程，由系统自动重建（会黑屏 1–2 秒）。
    RecreateCompositor {
        pid: Pid,
    },
    /// 记录命令行后正常关闭，再以用户身份重新启动。
    RestartApp {
        pid: Pid,
    },
}

/// 动作的性质，安全守卫按它分级放行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// 让进程更快（提高优先级、关闭节能限制）。
    Boost,
    /// 让进程让路（降低优先级、效率模式、限制到 E 核）。
    Demote,
    /// 恢复到默认状态。
    Restore,
    /// 重启或重建进程。
    Restart,
}

impl Action {
    pub fn pid(&self) -> Pid {
        match self {
            Self::SetPriority { pid, .. }
            | Self::SetEcoQos { pid, .. }
            | Self::SetCoreSet { pid, .. }
            | Self::RestartService { pid, .. }
            | Self::RecreateCompositor { pid }
            | Self::RestartApp { pid } => *pid,
        }
    }

    /// 调整优先级的方向取决于相对原始优先级，因此需要传入原始值。
    pub fn kind(&self, original_priority: PriorityClass) -> ActionKind {
        match self {
            Self::SetPriority { class, .. } => match class.rank().cmp(&original_priority.rank()) {
                std::cmp::Ordering::Greater => ActionKind::Boost,
                std::cmp::Ordering::Less => ActionKind::Demote,
                std::cmp::Ordering::Equal => ActionKind::Restore,
            },
            Self::SetEcoQos { state, .. } => match state {
                EcoQos::Default => ActionKind::Restore,
                EcoQos::Off => ActionKind::Boost,
                EcoQos::On => ActionKind::Demote,
            },
            Self::SetCoreSet { set, .. } => match set {
                CoreSet::All => ActionKind::Restore,
                CoreSet::Performance => ActionKind::Boost,
                CoreSet::Efficiency => ActionKind::Demote,
            },
            Self::RestartService { .. }
            | Self::RecreateCompositor { .. }
            | Self::RestartApp { .. } => ActionKind::Restart,
        }
    }

    /// 是否属于"任务管理器本身就能做"的操作。
    pub fn is_task_manager_equivalent(&self) -> bool {
        matches!(
            self,
            Self::SetPriority { .. } | Self::SetEcoQos { .. } | Self::SetCoreSet { .. }
        )
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExecError {
    #[error("被安全守卫拒绝：{0}")]
    Denied(String),
    #[error("没有权限（可能被反作弊或系统保护）")]
    AccessDenied,
    #[error("进程已退出")]
    Gone,
    #[error("执行失败：{0}")]
    Failed(String),
}

/// 动作的执行结果。重启类动作会返回新进程的 PID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecOutcome {
    pub new_pid: Option<Pid>,
}

/// 平台层实现的原始执行器。业务代码不直接使用它，而是通过安全守卫包装。
pub trait ActionExecutor: Send {
    fn execute(&self, action: &Action) -> Result<ExecOutcome, ExecError>;
}
