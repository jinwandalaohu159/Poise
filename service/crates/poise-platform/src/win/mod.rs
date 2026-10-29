//! 底层 Win32 封装：句柄、字符串、注册表、安全描述符、版本信息、COM、WMI、任务计划程序。

pub mod com;
pub mod console;
pub mod handle;
pub mod registry;
pub mod security;
pub mod task_scheduler;
pub mod version_info;
pub mod wide;
pub mod wmi;
