//! 只读观测：进程、系统指标、各逻辑处理器的累计时间、处理器实际性能与限速、CPU 拓扑、单个进程的显卡占用、
//! 服务表。不改变任何状态。

pub mod cpu_performance;
pub mod gpu;
pub mod process;
pub mod processors;
pub mod services;
pub mod system;
pub mod topology;
