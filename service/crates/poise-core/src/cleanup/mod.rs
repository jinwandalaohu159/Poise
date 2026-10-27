//! 清理：发现越用越占资源的进程，按类型安全地处理。

mod kernel_pool;
mod leak;
mod planner;
mod trend;

pub use kernel_pool::{KernelPoolConfig, KernelPoolWatch};
pub use leak::{LeakConfig, LeakDetector, LeakMetric, LeakSuspect, MetricThreshold};
pub use planner::{
    Advisory, CleanupConfig, CleanupPlan, CleanupReason, CleanupStep, PlanInput, plan,
};
