//! Windows 服务宿主：响应服务管理器的启动 / 停止。

use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use poise_platform::persist::paths;

use crate::config::RunConfig;
use crate::runtime::StopReason;
use crate::{logging, runtime};

pub const SERVICE_NAME: &str = "Poise";

define_windows_service!(ffi_service_main, service_main);

/// 由服务管理器调用（命令行参数 `run`）。
pub fn run_as_service() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let cfg = RunConfig::service();
    // 先建好数据目录（只有管理员可写），首次启动的日志才有地方写。失败时由 Engine 报告。
    let _ = paths::ensure_dir(&cfg.data_dir, true);
    logging::init(&cfg.data_dir, false);
    if let Err(e) = run(cfg) {
        log::error!("服务异常退出：{e:#}");
    }
    logging::flush();
}

fn status(state: ServiceState) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        // 关机时先收到"即将关机"（系统服务都还在）：趁散热接口（WMI）还能用时还原。
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::PRESHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn run(cfg: RunConfig) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop => {
            let _ = tx.send(StopReason::Requested);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Preshutdown | ServiceControl::Shutdown => {
            let _ = tx.send(StopReason::SystemShutdown);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    handle.set_service_status(status(ServiceState::Running))?;
    let result = runtime::run(cfg, rx, || {
        let _ = handle.set_service_status(status(ServiceState::StopPending));
    });
    // 异常退出时报告非零的退出码，服务管理器据此按"失败"处理（自动重启，见 `install`）。
    handle.set_service_status(ServiceStatus {
        exit_code: ServiceExitCode::ServiceSpecific(u32::from(result.is_err())),
        ..status(ServiceState::Stopped)
    })?;
    result
}
