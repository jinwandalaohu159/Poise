//! 底层 Win32 封装：句柄、字符串、注册表、安全描述符、版本信息。

pub mod console;
pub mod handle;
pub mod registry;
pub mod security;
pub mod version_info;
pub mod wide;
pub mod wmi;
