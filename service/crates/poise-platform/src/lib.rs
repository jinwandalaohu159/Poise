//! poise-core 中各接口的 Windows 实现。所有 Win32 调用都集中在这个 crate 里。
//!
//! - [`win`]：底层封装
//! - [`observe`]：只读观测
//! - [`act`]：改变运行状态
//! - [`settings`]：持久设置
//! - [`ipc`]：命名管道
//! - [`persist`]：数据目录与文件

pub mod act;
pub mod ipc;
pub mod observe;
pub mod persist;
pub mod settings;
pub mod win;
