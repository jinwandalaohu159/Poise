//! 文本日志：写到数据目录的 poise.log（超过 2MB 时换成 poise.1.log 继续写）；控制台模式同时输出到终端。
//! 写入不让写日志的线程等待（见 `poise_platform::logging`）。给界面看的结构化日志见 `poise_logbook`。

use std::path::Path;

use poise_platform::logging::{self, LevelFilter, LogFile};

pub fn init(data_dir: &Path, to_terminal: bool) {
    let level = if std::env::var_os("POISE_DEBUG").is_some() {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let file = LogFile {
        path: data_dir.join("poise.log"),
        max_bytes: 2 * 1024 * 1024,
    };
    logging::init(Some(file), level, to_terminal);
    logging::log_panics();
}

/// 等日志写完（服务结束前）。
pub fn flush() {
    logging::flush();
}
