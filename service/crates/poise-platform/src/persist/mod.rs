//! 数据目录与 JSON 文件持久化（[`writer`]：在后台线程里写；`secure_dir`：服务数据目录的权限）。

pub mod paths;
mod secure_dir;
pub mod storage;
pub mod writer;
