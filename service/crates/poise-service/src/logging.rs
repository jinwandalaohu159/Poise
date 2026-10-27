//! 日志：写到数据目录的 poise.log（超过 2MB 时清空重写）；控制台模式同时输出到终端。

use std::fs::OpenOptions;
use std::path::Path;

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

pub fn init(data_dir: &Path, to_terminal: bool) {
    let level = if std::env::var_os("POISE_DEBUG").is_some() {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let config = ConfigBuilder::new()
        .set_time_offset_to_local()
        .unwrap_or_else(|b| b)
        .set_target_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .build();
    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();
    if to_terminal {
        loggers.push(TermLogger::new(
            level,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ));
    }
    let path = data_dir.join("poise.log");
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let file = OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(&path);
    if let Ok(f) = file {
        loggers.push(WriteLogger::new(level, config, f));
    }
    let _ = CombinedLogger::init(loggers);
}
