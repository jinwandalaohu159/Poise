//! 日志：%LOCALAPPDATA%\Poise\agent.log（超过 1MB 时清空重写）。

use std::fs::OpenOptions;
use std::path::PathBuf;

use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn init() {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Poise")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("agent.log");
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let Ok(file) = OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(path)
    else {
        return;
    };
    let config = ConfigBuilder::new()
        .set_time_offset_to_local()
        .unwrap_or_else(|b| b)
        .set_target_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .build();
    let _ = WriteLogger::init(LevelFilter::Info, config, file);
}
