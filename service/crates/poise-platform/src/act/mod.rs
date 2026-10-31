//! 改变运行状态的操作：动作执行器（及调整前的原值）、内存与磁盘读写优先级、以用户身份启动、服务重启、
//! 自身轻量化。

pub mod executor;
pub mod launcher;
mod originals;
pub mod resource_priority;
pub mod self_tune;
pub mod service_control;
