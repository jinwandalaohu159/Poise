//! 数据目录与文件持久化：[`storage`]：JSON 文件；[`journal_file`]：变更日志；`atomic`：可靠地读写一个文件；
//! [`writer`]：在后台线程里写；`secure_dir`：服务数据目录的权限。

pub(crate) mod atomic;
pub mod journal_file;
pub mod paths;
mod secure_dir;
pub mod storage;
pub mod writer;
