//! 底层 Win32 封装：账户与 SID、句柄与要调整的进程、字符串、注册表、服务管理器、安全描述符、程序的版本信息、
//! 系统版本、COM、WMI、任务计划程序。

pub(crate) mod account;
pub mod com;
pub mod console;
pub mod env;
pub mod handle;
pub mod process;
pub mod registry;
pub mod scm;
pub mod security;
pub mod task_scheduler;
pub mod version;
pub mod version_info;
pub mod wide;
pub mod wmi;
