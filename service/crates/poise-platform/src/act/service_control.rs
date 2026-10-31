//! 安全地重启一个 Windows 服务：停止 → 等待 → 启动 → 等待。

use std::time::{Duration, Instant};

use poise_core::action::ExecError;
use poise_core::model::Pid;
use windows::Win32::System::Services::{
    ControlService, SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOP,
    SERVICE_STOPPED, StartServiceW,
};

use crate::win::handle::map_error;
use crate::win::scm::{ScHandle, open_manager, open_service, running_dependents, status};

fn wait_for(
    svc: &ScHandle,
    state: u32,
    timeout: Duration,
) -> Result<SERVICE_STATUS_PROCESS, ExecError> {
    let start = Instant::now();
    loop {
        let s = status(svc).map_err(|e| map_error(&e))?;
        if s.dwCurrentState.0 == state {
            return Ok(s);
        }
        if start.elapsed() >= timeout {
            return Err(ExecError::Failed("等待服务状态超时".into()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 停止再启动一个服务，返回新的进程 PID。有正在运行的依赖服务时不重启（会连带停掉它们）：
/// 计划里的依赖数可能是一会儿之前数的，这里再核对一次。
pub fn restart(name: &str) -> Result<Option<Pid>, ExecError> {
    let scm = open_manager(SC_MANAGER_CONNECT).map_err(|e| map_error(&e))?;
    if running_dependents(&scm, name).map_err(|e| map_error(&e))? > 0 {
        return Err(ExecError::Failed("有正在运行的服务依赖它".into()));
    }
    let svc = open_service(
        &scm,
        name,
        SERVICE_STOP | SERVICE_START | SERVICE_QUERY_STATUS,
    )
    .map_err(|e| map_error(&e))?;
    let mut status = SERVICE_STATUS::default();
    // SAFETY: 句柄有效，状态结构为本地变量。
    unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) }
        .map_err(|e| map_error(&e))?;
    wait_for(&svc, SERVICE_STOPPED.0, Duration::from_secs(15))?;
    // SAFETY: 无启动参数。
    unsafe { StartServiceW(svc.0, None) }.map_err(|e| map_error(&e))?;
    let s = wait_for(&svc, SERVICE_RUNNING.0, Duration::from_secs(15))?;
    Ok((s.dwProcessId != 0).then_some(s.dwProcessId))
}
