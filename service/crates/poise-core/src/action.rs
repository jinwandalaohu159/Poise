//! 所有会改变系统运行状态的动作。动作只能交给 [`crate::safety::GuardedExecutor`] 执行。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{Pid, PriorityClass, ProcessKey};

/// 效率模式（EcoQoS）状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EcoQos {
    /// 交给系统决定（不干预）。
    #[default]
    Default,
    /// 明确关闭节能限制：不允许系统对这个进程降频、放宽计时器精度（Windows 会对后台、
    /// 最小化、被遮挡的程序自动这样做，游戏切出去、后台播放声音时会因此卡顿）。
    Off,
    /// 开启效率模式：优先放到 E 核并降频，不理会它请求的高精度计时器。
    On,
}

/// 内存与磁盘读写的优先级，一起调整。低时：内存紧张时系统先回收它的内存页，磁盘忙时先满足别人的读写。
/// Windows 不会因为程序在后台就降低这两项，后台的更新、索引、同步大量读写磁盘时，前台和游戏照样被拖慢。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ResourcePriority {
    /// 系统默认（正常）。
    #[default]
    Default,
    /// 低：先回收它的内存页、先满足别人的磁盘读写，保护正在用的程序。
    Low,
}

/// 进程可用的核心集合。
///
/// 核心按"核心域"划分（见平台层 `CpuTopology::domains`）：同一个域里的核心效率等级相同、共用一块缓存，
/// 按适合跑重点程序的程度排序。第 0 个是主域：大小核处理器的大核，多 CCD 锐龙里缓存最大的那个 CCD；
/// 其余是副域：小核（按共用的二级缓存分组）、另一个 CCD。只有一个域的处理器不限制核心。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CoreSet {
    /// 不限制。
    #[default]
    All,
    /// 主域（大核）。
    Performance,
    /// 全部副域（小核）。
    Efficiency,
    /// 具体的几个核心域（第 n 位对应第 n 个域）：调度按负载和缓存分配的结果。
    Domains(u16),
}

impl CoreSet {
    /// 只用副域（让开了主域，例如让到小核）。
    pub fn is_efficiency(self) -> bool {
        match self {
            Self::Efficiency => true,
            Self::Domains(m) => m != 0 && m & 1 == 0,
            _ => false,
        }
    }

    /// 用主域、并且有限制（重点程序放在大核上，需要时加上几个副域）。
    pub fn is_performance(self) -> bool {
        match self {
            Self::Performance => true,
            Self::Domains(m) => m & 1 != 0,
            _ => false,
        }
    }

    /// 在有 `domains` 个核心域的处理器上用到哪些域（第 n 位对应第 n 个域）；不限制时是全部。
    pub fn domain_mask(self, domains: usize) -> u16 {
        let all = if domains >= 16 {
            u16::MAX
        } else {
            (1u16 << domains) - 1
        };
        let mask = match self {
            Self::All => all,
            Self::Performance => 1,
            Self::Efficiency => all & !1,
            Self::Domains(m) => m,
        } & all;
        if mask == 0 { all } else { mask }
    }
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
    SetResourcePriority {
        pid: Pid,
        priority: ResourcePriority,
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
            | Self::SetResourcePriority { pid, .. }
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
            Self::SetCoreSet { set, .. } => {
                if set.is_performance() {
                    ActionKind::Boost
                } else if set.is_efficiency() {
                    ActionKind::Demote
                } else {
                    ActionKind::Restore
                }
            }
            Self::SetResourcePriority { priority, .. } => match priority {
                ResourcePriority::Default => ActionKind::Restore,
                ResourcePriority::Low => ActionKind::Demote,
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
            Self::SetPriority { .. }
                | Self::SetEcoQos { .. }
                | Self::SetCoreSet { .. }
                | Self::SetResourcePriority { .. }
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
pub trait ActionExecutor: Send + Sync {
    /// 对 `target` 执行动作。执行前核对进程还是同一个（创建时间一致，为 0 时不核对）：
    /// 进程号已被新进程复用时返回 [`ExecError::Gone`]，绝不动到别的程序。
    fn execute(&self, target: ProcessKey, action: &Action) -> Result<ExecOutcome, ExecError>;
}

#[cfg(test)]
mod tests {
    use super::CoreSet;

    #[test]
    fn core_sets_map_to_the_domains_they_use() {
        assert_eq!(CoreSet::All.domain_mask(3), 0b111);
        assert_eq!(CoreSet::Performance.domain_mask(3), 0b001);
        assert_eq!(CoreSet::Efficiency.domain_mask(3), 0b110);
        assert_eq!(CoreSet::Domains(0b011).domain_mask(3), 0b011);
        // 只有一个域：怎样都是不限制。
        assert_eq!(CoreSet::Efficiency.domain_mask(1), 0b1);
        assert_eq!(CoreSet::All.domain_mask(16), u16::MAX);
    }
}
