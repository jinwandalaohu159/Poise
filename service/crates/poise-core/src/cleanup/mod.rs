//! 清理：发现越用越占资源的进程，按类型安全地处理。

mod growth;
mod kernel_pool;
mod leak;
mod overview;
mod planner;
mod trend;

pub use growth::{
    BootBaseline, Change, Grown, GrowthConfig, GrowthLimit, GrowthMetric, GrowthWatch,
    SystemTotals, TotalChange, TotalItem, compare_with_boot,
};
pub use kernel_pool::{KernelPoolConfig, KernelPoolWatch};
pub use leak::{LeakConfig, LeakDetector, LeakMetric, LeakSuspect, MetricThreshold};
pub use overview::{Issue, OverviewInput, ProcessGroup, Remedy, overview};
pub use planner::{
    Advisory, CleanupConfig, CleanupPlan, CleanupReason, CleanupStep, PlanInput, plan,
};
