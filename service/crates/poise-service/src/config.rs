//! 运行配置。

use std::path::PathBuf;

use poise_platform::persist::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// 开发模式：以当前用户在控制台运行。
    Console,
    /// 作为 Windows 服务以 SYSTEM 运行。
    Service,
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub mode: RunMode,
    /// 观察模式：只判断、只记录，不执行任何动作，也不修改任何设置。
    pub dry_run: bool,
    pub data_dir: PathBuf,
}

impl RunConfig {
    pub fn console(live: bool) -> Self {
        Self {
            mode: RunMode::Console,
            dry_run: !live,
            data_dir: paths::dev_data_dir(),
        }
    }

    pub fn service() -> Self {
        Self {
            mode: RunMode::Service,
            dry_run: false,
            data_dir: paths::service_data_dir(),
        }
    }
}
