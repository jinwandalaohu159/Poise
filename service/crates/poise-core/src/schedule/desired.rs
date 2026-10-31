//! 进程可以处于的调度状态（优先级、节能、核心、内存与磁盘优先级的组合）。什么情况用哪个见 [`super::policy`]。

use crate::action::{CoreSet, EcoQos, ResourcePriority};
use crate::model::PriorityClass;

/// 一个进程期望的调度状态。`priority` 为 None 表示保持原始优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Desired {
    pub priority: Option<PriorityClass>,
    pub eco: EcoQos,
    pub cores: CoreSet,
    /// 内存与磁盘读写的优先级。
    pub resources: ResourcePriority,
}

impl Desired {
    pub const NEUTRAL: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::All,
        resources: ResourcePriority::Default,
    };

    /// 在后台让出一部分性能：低于正常，内存紧张时先回收它的内存、磁盘忙时先满足别人。核心不限、不节能——
    /// 正在用的程序需要时优先拿到处理器，其余时间空闲的核心照样给它用，后台的工作不白白变慢。
    pub const LOWERED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Default,
        cores: CoreSet::All,
        resources: ResourcePriority::Low,
    };

    /// 在后台完整让路：让开主域（小核）、低于正常、节能；内存紧张时先回收它的内存，保护正在用的程序。
    /// 用于有重点程序、处理器发热、用电池时，以及用户设为"低优先级"的程序。
    pub const YIELDED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Low,
    };

    /// 需求高时，在后台忙、但负载轻的程序（达不到需求一般时的门槛）：让开主域、低于正常，不节能、
    /// 不降内存与磁盘优先级。手柄与按键映射、头部追踪、录像、副屏上的视频这类程序负载不高，却对时延敏感：
    /// 节能会放宽它们的计时器精度、压低频率，输入跟着变迟、画面跟着掉帧（Windows 自己也只对没有可见窗口的
    /// 程序这样做）。挪开它们已经足够让出大核。
    pub const NUDGED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Default,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    /// 用户标记的游戏：在大核上运行，优先级提高一级（高于正常，见 `reconcile::effective_priority`），
    /// 不让系统自动节能限速。
    pub const GAME: Self = Self {
        priority: Some(PriorityClass::AboveNormal),
        eco: EcoQos::Off,
        cores: CoreSet::Performance,
        resources: ResourcePriority::Default,
    };

    /// 自动识别的、正在玩的游戏：在大核上运行，不让系统自动节能限速。
    pub const PLAYING: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::Performance,
        resources: ResourcePriority::Default,
    };

    /// 正在发声、用户要求保持快的程序：不提优先级、不限核心，只是不让系统自动节能限速。
    pub const PROTECTED: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::All,
        resources: ResourcePriority::Default,
    };

    /// 正在发声、有重点程序时又在后台忙的程序：只让到小核。不降优先级、不节能——声音线程有自己的
    /// 实时优先级，在小核上照样流畅；大核留给重点程序。
    pub const BACKSEAT: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    /// 有重点程序时，没在用的程序：只让到小核，大核留给重点程序。不降优先级、不节能——它们没在忙，
    /// 只是零碎的工作不再打断大核上的重点程序；需要时（回到前台、发声）立即恢复。
    pub const ASIDE: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    /// 有重点程序时，Windows 自己的维护程序（更新安装、搜索索引等）：挪到副域，内存与磁盘优先级降低。
    /// 不改优先级、不节能——它们是系统的一部分，只是不占重点程序用着的核心、不和它抢磁盘。
    pub const MAINTENANCE_ASIDE: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Low,
    };

    /// 低优先级程序到了前台：仍在小核，但不节能，界面照样能点。
    pub const LOW_IN_FOREGROUND: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Off,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    pub fn is_neutral(&self) -> bool {
        *self == Self::NEUTRAL
    }
}
