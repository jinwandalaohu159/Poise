//! 动作执行器：把 poise-core 的动作翻译成 Win32 调用。
//!
//! 只申请完成动作所需的最小权限（与任务管理器相同），被拒绝时直接返回，不重试、不绕过。
//! 业务代码不直接使用它，而是经过 `poise_core::safety::GuardedExecutor`。

use std::ffi::c_void;

use poise_core::action::{Action, ActionExecutor, CoreSet, EcoQos, ExecError, ExecOutcome};
use poise_core::model::{Pid, PriorityClass};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_SET_INFORMATION, PROCESS_SET_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, ProcessPowerThrottling, SetPriorityClass, SetProcessDefaultCpuSets,
    SetProcessInformation, TerminateProcess, WaitForSingleObject,
};

use crate::act::launcher::Launcher;
use crate::act::resource_priority;
use crate::act::service_control;
use crate::observe::process::cmdline;
use crate::observe::topology::CpuTopology;
use crate::win::handle::{map_error, open_process};

pub struct WinExecutor {
    topology: CpuTopology,
    launcher: Box<dyn Launcher>,
}

impl WinExecutor {
    pub fn new(topology: CpuTopology, launcher: Box<dyn Launcher>) -> Self {
        Self { topology, launcher }
    }
}

impl ActionExecutor for WinExecutor {
    fn execute(&self, action: &Action) -> Result<ExecOutcome, ExecError> {
        match action {
            Action::SetPriority { pid, class } => set_priority(*pid, *class),
            Action::SetEcoQos { pid, state } => set_eco_qos(*pid, *state),
            Action::SetCoreSet { pid, set } => self.set_cores(*pid, *set),
            Action::SetResourcePriority { pid, priority } => {
                resource_priority::set(*pid, *priority).map(|()| ExecOutcome::default())
            }
            Action::RestartService { service, .. } => {
                service_control::restart(service).map(|new_pid| ExecOutcome { new_pid })
            }
            Action::RecreateCompositor { pid } => {
                terminate(*pid)?;
                Ok(ExecOutcome::default())
            }
            Action::RestartApp { pid } => self.restart_app(*pid),
        }
        .inspect_err(|e| log::debug!("执行 {action:?} 失败：{e}"))
    }
}

fn set_priority(pid: Pid, class: PriorityClass) -> Result<ExecOutcome, ExecError> {
    let flags: PROCESS_CREATION_FLAGS = match class {
        PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
        // 永远不设置实时优先级，它可能让整个系统失去响应。
        PriorityClass::Realtime => return Err(ExecError::Failed("不允许实时优先级".into())),
    };
    let h = open_process(pid, PROCESS_SET_INFORMATION)?;
    // SAFETY: 句柄具有 PROCESS_SET_INFORMATION 权限。
    unsafe { SetPriorityClass(h.raw(), flags) }.map_err(|e| map_error(&e))?;
    Ok(ExecOutcome::default())
}

fn set_eco_qos(pid: Pid, state: EcoQos) -> Result<ExecOutcome, ExecError> {
    let speed = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
    let timer = PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;
    let (control, on) = match state {
        EcoQos::Default => (0, 0),
        // 两项都明确"不限制"：系统不会自动给它降频，也不会放宽它请求的计时器精度。
        EcoQos::Off => (speed | timer, 0),
        EcoQos::On => (speed, speed),
    };
    let info = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control,
        StateMask: on,
    };
    let h = open_process(pid, PROCESS_SET_INFORMATION)?;
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        SetProcessInformation(
            h.raw(),
            ProcessPowerThrottling,
            &info as *const _ as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(|e| map_error(&e))?;
    Ok(ExecOutcome::default())
}

impl WinExecutor {
    fn set_cores(&self, pid: Pid, set: CoreSet) -> Result<ExecOutcome, ExecError> {
        if !self.topology.is_hybrid() {
            return Ok(ExecOutcome::default());
        }
        let ids = self.topology.ids_for(set);
        let h = open_process(pid, PROCESS_SET_LIMITED_INFORMATION)?;
        // SAFETY: ids 为 None 时清除默认 CPU Set（即不限制）。
        let ok = unsafe { SetProcessDefaultCpuSets(h.raw(), ids) };
        if ok.as_bool() {
            Ok(ExecOutcome::default())
        } else {
            Err(map_error(&windows::core::Error::from_thread()))
        }
    }

    /// 先记录启动信息，再结束进程，最后以原用户身份重新启动。
    fn restart_app(&self, pid: Pid) -> Result<ExecOutcome, ExecError> {
        let spec = cmdline::capture(pid)?;
        let mut session_id = 0u32;
        // SAFETY: 输出到本地变量。
        unsafe { ProcessIdToSessionId(pid, &mut session_id) }.map_err(|e| map_error(&e))?;
        if session_id == 0 {
            return Err(ExecError::Failed("不重启会话 0 的进程".into()));
        }
        terminate(pid)?;
        let new_pid = self
            .launcher
            .launch(&spec, session_id)
            .or_else(|_| self.launcher.launch(&spec, session_id))?;
        Ok(ExecOutcome {
            new_pid: Some(new_pid),
        })
    }
}

/// 结束进程并等待它真正退出（最多 5 秒）。
fn terminate(pid: Pid) -> Result<(), ExecError> {
    let h = open_process(pid, PROCESS_TERMINATE | PROCESS_SYNCHRONIZE)?;
    // SAFETY: 句柄具有 PROCESS_TERMINATE 权限。
    unsafe { TerminateProcess(h.raw(), 1) }.map_err(|e| map_error(&e))?;
    // SAFETY: 句柄具有 SYNCHRONIZE 权限。
    if unsafe { WaitForSingleObject(h.raw(), 5000) } != WAIT_OBJECT_0 {
        return Err(ExecError::Failed("进程没有在 5 秒内退出".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command};

    use super::*;
    use crate::act::launcher::DirectLauncher;

    /// 只对测试自己启动的子进程执行动作，不碰系统里的任何其他程序。
    struct Sandbox(Child);

    impl Sandbox {
        fn spawn() -> Self {
            Self(
                Command::new("cmd.exe")
                    .args(["/c", "ping", "-n", "30", "127.0.0.1", ">nul"])
                    .spawn()
                    .expect("spawn test child"),
            )
        }
        fn pid(&self) -> Pid {
            self.0.id()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn executor() -> WinExecutor {
        WinExecutor::new(CpuTopology::detect(), Box::new(DirectLauncher))
    }

    #[test]
    fn adjusts_priority_eco_and_cores_on_own_child() {
        let child = Sandbox::spawn();
        let ex = executor();
        let pid = child.pid();
        for action in [
            Action::SetPriority {
                pid,
                class: PriorityClass::BelowNormal,
            },
            Action::SetEcoQos {
                pid,
                state: EcoQos::On,
            },
            Action::SetCoreSet {
                pid,
                set: CoreSet::Efficiency,
            },
            Action::SetCoreSet {
                pid,
                set: CoreSet::All,
            },
            Action::SetEcoQos {
                pid,
                state: EcoQos::Default,
            },
            Action::SetPriority {
                pid,
                class: PriorityClass::Normal,
            },
        ] {
            ex.execute(&action)
                .unwrap_or_else(|e| panic!("{action:?}: {e}"));
        }
    }

    #[test]
    fn realtime_priority_is_refused() {
        let child = Sandbox::spawn();
        let r = executor().execute(&Action::SetPriority {
            pid: child.pid(),
            class: PriorityClass::Realtime,
        });
        assert!(matches!(r, Err(ExecError::Failed(_))));
    }

    #[test]
    fn restart_app_relaunches_own_child() {
        let child = Sandbox::spawn();
        let out = executor()
            .execute(&Action::RestartApp { pid: child.pid() })
            .expect("restart");
        let new_pid = out.new_pid.expect("new pid");
        assert_ne!(new_pid, child.pid());
        // 清理重新启动出来的测试进程。
        let _ = terminate(new_pid);
    }

    #[test]
    fn missing_process_reports_gone() {
        let r = executor().execute(&Action::SetPriority {
            pid: 0xFFFF_FFF0,
            class: PriorityClass::Normal,
        });
        assert_eq!(r, Err(ExecError::Gone));
    }
}
