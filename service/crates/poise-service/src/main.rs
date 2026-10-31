//! Poise 后台服务。
//!
//! 用法：
//!   poise-service console [--live]   开发模式：在控制台运行。默认为观察模式（只判断不执行），
//!                                    加 --live 才真正执行。
//!   poise-service install            安装并启动 Windows 服务（需要管理员）
//!   poise-service uninstall          停止并卸载服务，还原所有修改（需要管理员）
//!   poise-service run                由服务管理器调用

mod config;
mod engine;
mod gpu_feed;
mod host;
mod install;
mod ipc_server;
mod logging;
mod power_events;
mod power_reader;
mod runtime;
mod sensor_feed;

use std::sync::mpsc;

use config::RunConfig;

const USAGE: &str = "用法：poise-service <console [--live] | install | uninstall | run>";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("console") => console(args.iter().any(|a| a == "--live")),
        Some("run") => Ok(host::run_as_service()?),
        Some("install") => install::install(),
        Some("uninstall") => install::uninstall(),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn console(live: bool) -> anyhow::Result<()> {
    let cfg = RunConfig::console(live);
    std::fs::create_dir_all(&cfg.data_dir)?;
    logging::init(&cfg.data_dir, true);
    let (tx, rx) = mpsc::channel();
    poise_platform::win::console::on_stop(move || {
        let _ = tx.send(runtime::Signal::Stop(runtime::StopReason::Requested));
    })?;
    log::info!("数据目录：{}（按 Ctrl+C 退出）", cfg.data_dir.display());
    let result = runtime::run(cfg, rx, || {});
    logging::flush();
    result
}
