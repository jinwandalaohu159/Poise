//! Windows 服务宿主：响应服务管理器的启动 / 停止，转达熄屏、待机、醒来（见 `engine::standby`）。

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
use crate::runtime::{Signal, StopReason};
use crate::{logging, power_events, runtime};

pub use poise_protocol::SERVICE_NAME;

define_windows_service!(ffi_service_main, service_main);

/// 由服务管理器调用（命令行参数 `run`）。
pub fn run_as_service() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let cfg = RunConfig::service();
    // 先建好数据目录、收紧权限，再打开里面的任何文件（日志也在里面）：收紧之前打开的话，别人事先放的
    // 链接、伪造的文件已经以 SYSTEM 打开了。处理了什么等日志打开后再记。
    let secured = paths::ensure_dir(&cfg.data_dir, true);
    logging::init(&cfg.data_dir, false);
    match secured {
        Ok(notes) => notes.iter().for_each(|n| log::warn!("{n}")),
        Err(e) => {
            // 收紧不了就不能信里面的记录（变更日志会以 SYSTEM 写回注册表）：不启动。
            log::error!("收紧数据目录权限失败，不按里面的记录做事，服务不启动：{e}");
            logging::flush();
            return;
        }
    }
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
            ServiceControlAccept::STOP
                | ServiceControlAccept::PRESHUTDOWN
                | ServiceControlAccept::POWER_EVENT
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        // 停止时要撤销调整、还原硬件模式（电源最多等 10 秒、散热 5 秒、一键清理 10 秒……）：预计时间给足，
        // 控制程序（net stop、安装程序）不会等不及就当成停止失败。
        wait_hint: Duration::from_secs(if state == ServiceState::StopPending {
            60
        } else {
            10
        }),
        process_id: None,
    }
}

fn run(cfg: RunConfig) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop => {
            let _ = tx.send(Signal::Stop(StopReason::Requested));
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Preshutdown | ServiceControl::Shutdown => {
            let _ = tx.send(Signal::Stop(StopReason::SystemShutdown));
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::PowerEvent(param) => {
            if let Some(change) = power_events::power_change(param) {
                let _ = tx.send(Signal::Power(change));
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    handle.set_service_status(status(ServiceState::Running))?;
    let _display = power_events::subscribe_display(&handle);
    let result = runtime::run(cfg, rx, || {
        let _ = handle.set_service_status(status(ServiceState::StopPending));
    });
    // 异常退出时报告非零的退出码，服务管理器据此按"失败"处理（自动重启，见 `install`）。正常停止必须报告
    // 0：`ServiceSpecific(0)` 也算失败（Win32 退出码是"服务特定错误"），停止后 5 秒又会被拉起来。
    let exit_code = if result.is_err() {
        ServiceExitCode::ServiceSpecific(1)
    } else {
        ServiceExitCode::Win32(0)
    };
    handle.set_service_status(ServiceStatus {
        exit_code,
        ..status(ServiceState::Stopped)
    })?;
    result
}
