//! 文本日志（服务、托盘代理、更新助手共用）：按大小轮换的日志文件（见 [`rotating`]），开发时同时输出到终端。
//!
//! 写日志的线程只把这一行交给写入线程（见 [`writer`]），从不等文件：服务的引擎线程常常拿着引擎锁写日志，
//! 写入线程和 Poise 其余部分一样以低优先级运行，要是让引擎线程排队等文件，整个调度就跟着停下。
//! 写入线程每批只写一次文件。程序崩溃时把原因写进日志（见 [`log_panics`]）。

mod line;
mod rotating;
mod writer;

use std::path::PathBuf;

pub use log::LevelFilter;

/// 日志文件：路径和最大字节数（写满后改名为 `.1.log`，连同上一份最多占两倍）。
pub struct LogFile {
    pub path: PathBuf,
    pub max_bytes: u64,
}

/// 设为全局日志。文件打不开时只输出到终端（或什么都不写）。
pub fn init(file: Option<LogFile>, level: LevelFilter, to_terminal: bool) {
    let file = file.and_then(|f| rotating::RotatingFile::open(f.path, f.max_bytes));
    let logger = writer::start(file, to_terminal, level);
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
}

/// 等写入线程把交来的都写完（程序结束前），最多等一秒。
pub fn flush() {
    log::logger().flush();
}

/// 程序崩溃（panic）时把原因和位置写进日志：发布版遇到 panic 直接结束进程，不留下任何别的痕迹。
pub fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        log::error!(
            "程序崩溃（线程 {}）：{info}",
            thread.name().unwrap_or("未命名")
        );
        flush();
    }));
}
