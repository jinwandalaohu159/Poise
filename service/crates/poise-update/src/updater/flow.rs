//! 更新流程（后台线程）：查询 → 下载并校验 → 安装 → 等服务启动 → 启动托盘代理。
//! 每一步都把进度写进共享状态，进度窗口定时读取。

use std::path::Path;
use std::time::Instant;

use super::args::Args;
use super::download;
use super::progress::{Failure, Outcome, Shared, Stage};
use super::setup;

/// 安装程序结束时，"安装并启动"这一段的进度；之后是等服务（到 0.95），最后启动托盘代理。
pub const INSTALLED_AT: f32 = 0.8;
const SERVICE_UP_TO: f32 = 0.95;

pub fn run(args: &Args, shared: &Shared) -> Outcome {
    match update(args, shared) {
        Ok(outcome) => outcome,
        Err(failure) => {
            log::warn!("更新没有完成：{failure:?}");
            Outcome::Failed(failure)
        }
    }
}

fn update(args: &Args, shared: &Shared) -> Result<Outcome, Failure> {
    let installed = installed_version(&args.dir);
    let Some(release) = poise_update::latest_release().map_err(Failure::Network)? else {
        return Ok(Outcome::UpToDate);
    };
    log::info!("已安装 {installed:?}，最新 {}", release.version);
    shared.set_version(&release.version);
    if installed
        .as_deref()
        .is_some_and(|v| !poise_update::is_newer(&release.version, v))
    {
        return Ok(Outcome::UpToDate);
    }

    let installer = download::fetch(&release, &args.updates_dir(), shared)?;
    if shared.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    shared.set_stage(Stage::Installing {
        since: Instant::now(),
    });
    let installed = setup::run_installer(&installer.path, shared.owner());
    drop(installer);
    match installed {
        Ok(()) => {}
        // 安装程序没有运行：什么都没变，托盘代理还在。
        Err(f @ (Failure::Declined | Failure::Launch(_))) => return Err(f),
        // 安装失败会回滚并恢复服务；托盘代理已被安装程序关闭，重新启动它。
        Err(f) => {
            setup::start_agent(&args.dir);
            return Err(f);
        }
    }

    shared.set_stage(Stage::Starting {
        fraction: INSTALLED_AT,
    });
    setup::wait_for_service(|waited| {
        shared.set_stage(Stage::Starting {
            fraction: INSTALLED_AT + (SERVICE_UP_TO - INSTALLED_AT) * waited,
        });
    })?;
    setup::start_agent(&args.dir);
    shared.set_stage(Stage::Starting { fraction: 1.0 });
    Ok(Outcome::Updated)
}

/// 安装目录里的 version.txt（构建时写入）。
fn installed_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("version.txt"))
        .ok()
        .map(|s| s.trim().to_string())
}
