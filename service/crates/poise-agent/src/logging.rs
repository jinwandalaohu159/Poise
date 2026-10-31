//! 日志：%LOCALAPPDATA%\Poise\agent.log（超过 1MB 时换成 agent.1.log 继续写）。

use std::path::PathBuf;

use poise_platform::logging::{self, LevelFilter, LogFile};

pub fn init() {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Poise")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = LogFile {
        path: dir.join("agent.log"),
        max_bytes: 1024 * 1024,
    };
    logging::init(Some(file), LevelFilter::Info, false);
    logging::log_panics();
}
