//! 更新助手的各部分：参数、共享的进度、文字、下载与校验、安装与启动、流程、显示内容、进度窗口。

mod args;
mod download;
mod flow;
mod progress;
mod setup;
mod text;
mod verify;
mod view;
mod window;

use std::sync::Arc;

use args::Args;
use progress::Shared;
use text::Text;

pub fn run() {
    let Some(args) = Args::parse(std::env::args().skip(1)) else {
        // 只由界面启动；参数不对说明不是正常的调用，不显示任何东西。
        return;
    };
    init_logging(&args);
    log::info!(
        "更新助手 {} 启动，安装目录 {}",
        poise_update::CURRENT,
        args.dir.display()
    );
    let text = Text::new(args.lang.as_deref());
    let shared = Arc::new(Shared::default());
    let worker = shared.clone();
    std::thread::spawn(move || {
        let outcome = flow::run(&args, &worker);
        worker.finish(outcome);
    });
    window::show(&text, &shared);
    // 窗口关闭即结束（下载中取消时，下载线程随进程结束）。
}

fn init_logging(args: &Args) {
    let _ = std::fs::create_dir_all(&args.data_dir);
    if let Ok(file) = std::fs::File::create(args.data_dir.join("updater.log")) {
        let config = simplelog::ConfigBuilder::new()
            .set_time_offset_to_local()
            .unwrap_or_else(|b| b)
            .set_target_level(log::LevelFilter::Off)
            .set_thread_level(log::LevelFilter::Off)
            .build();
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, config, file);
    }
}
