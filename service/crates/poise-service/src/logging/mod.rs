//! 文本日志：写到数据目录的 poise.log（超过 2MB 时换成 poise.1.log 继续写，见 [`rotating`]）；
//! 控制台模式同时输出到终端。给界面看的结构化日志见 `crate::events`。

mod rotating;

use std::path::Path;

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};

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
    if let Some(file) = rotating::RotatingFile::open(data_dir.join("poise.log")) {
        loggers.push(WriteLogger::new(level, config, file));
    }
    let _ = CombinedLogger::init(loggers);
}
