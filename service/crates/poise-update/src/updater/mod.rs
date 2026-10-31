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

use poise_platform::win::wide::WideString;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;

use args::Args;
use progress::Shared;
use text::Text;

pub fn run() {
    let Some(args) = Args::parse(std::env::args().skip(1)) else {
        // 只由界面启动；参数不对说明不是正常的调用，不显示任何东西。
        return;
    };
    init_logging(&args);
    // 同一时间只有一个更新助手（两处点了"更新"、另一个用户也在更新）：两个同时下载、同时运行安装程序
    // 会互相打断。互斥体在进程整个生命周期内持有（不关闭句柄）。
    let name = WideString::new(r"Global\Poise.Updater");
    // SAFETY: 名称以 NUL 结尾；随后立即读取它的错误码。
    let _instance = unsafe { CreateMutexW(None, true, name.as_pcwstr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log::info!("已经有一个更新助手在运行，这个退出");
        return;
    }
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
    use poise_platform::logging::{self, LevelFilter, LogFile};
    let _ = std::fs::create_dir_all(&args.data_dir);
    let file = LogFile {
        path: args.data_dir.join("updater.log"),
        max_bytes: 256 * 1024,
    };
    logging::init(Some(file), LevelFilter::Info, false);
    logging::log_panics();
}
