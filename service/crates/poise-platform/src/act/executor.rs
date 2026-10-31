//! 动作执行器：把 poise-core 的动作翻译成 Win32 调用。
//!
//! 只申请完成动作所需的最小权限（与任务管理器相同），被拒绝时直接返回，不重试、不绕过。
//! 业务代码不直接使用它，而是经过 `poise_core::safety::GuardedExecutor`。

use poise_core::action::{Action, ActionExecutor, CoreSet, EcoQos, ExecError, ExecOutcome};
use poise_core::model::{PriorityClass, ProcessKey};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, SetPriorityClass, TerminateProcess,
    WaitForSingleObject,
};

use crate::act::launcher::Launcher;
use crate::act::originals::{Originals, Undo};
use crate::act::resource_priority;
use crate::act::service_control;
use crate::observe::process::cmdline;
use crate::observe::topology::CpuTopology;
use crate::win::handle::map_error;
use crate::win::process::{Target, Throttling};

pub struct WinExecutor {
    topology: CpuTopology,
    launcher: Box<dyn Launcher>,
    /// 调整前的节能设置与核心范围（撤销时还原，见 [`super::originals`]）。
    eco: Originals<Throttling>,
    cores: Originals<Vec<u32>>,
}

impl WinExecutor {
    pub fn new(topology: CpuTopology, launcher: Box<dyn Launcher>) -> Self {
        Self {
            topology,
            launcher,
            eco: Originals::default(),
            cores: Originals::default(),
        }
    }
}

/// 进程还在（进程号没有被别的进程复用）。
fn alive(key: ProcessKey) -> bool {
    Target::open(key, PROCESS_QUERY_LIMITED_INFORMATION).is_ok()
}

impl ActionExecutor for WinExecutor {
    fn execute(&self, target: ProcessKey, action: &Action) -> Result<ExecOutcome, ExecError> {
        debug_assert_eq!(target.pid, action.pid());
        match action {
            Action::SetPriority { class, .. } => set_priority(target, *class),
            Action::SetEcoQos { state, .. } => self.set_eco_qos(target, *state),
            Action::SetCoreSet { set, .. } => self.set_cores(target, *set),
            Action::SetResourcePriority { priority, .. } => {
                resource_priority::set(target, *priority).map(|()| ExecOutcome::default())
            }
            Action::RestartService { service, .. } => {
                service_control::restart(service).map(|new_pid| ExecOutcome { new_pid })
            }
            Action::RecreateCompositor { .. } => {
                terminate(target)?;
                Ok(ExecOutcome::default())
            }
            Action::RestartApp { .. } => self.restart_app(target),
        }
        .inspect_err(|e| {
            if *e == ExecError::Gone {
                self.eco.forget(target);
                self.cores.forget(target);
            }
            log::debug!("执行 {action:?} 失败：{e}");
        })
    }
}

fn set_priority(target: ProcessKey, class: PriorityClass) -> Result<ExecOutcome, ExecError> {
    let flags: PROCESS_CREATION_FLAGS = match class {
        PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
        // 永远不设置实时优先级，它可能让整个系统失去响应。
        PriorityClass::Realtime => return Err(ExecError::Failed("不允许实时优先级".into())),
    };
    let h = Target::open(target, PROCESS_SET_INFORMATION)?;
    // SAFETY: 句柄具有 PROCESS_SET_INFORMATION 权限。
    unsafe { SetPriorityClass(h.raw(), flags) }.map_err(|e| h.failure(&e))?;
    Ok(ExecOutcome::default())
}

impl WinExecutor {
    fn set_eco_qos(&self, target: ProcessKey, state: EcoQos) -> Result<ExecOutcome, ExecError> {
        let speed = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
        let timer = PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;
        let wanted = match state {
            EcoQos::Default => None,
            // 两项都明确"不限制"：系统不会自动给它降频，也不会放宽它请求的计时器精度。
            EcoQos::Off => Some(Throttling {
                control: speed | timer,
                state: 0,
            }),
            // 两项都开启：降频，并且不理会它请求的高精度计时器——后台程序请求 1 毫秒计时器会让整机的
            // 计时器中断变密（Windows 只对没有可见窗口的程序自动这样做）。
            EcoQos::On => Some(Throttling {
                control: speed | timer,
                state: speed | timer,
            }),
        };
        let h = Target::open(target, PROCESS_SET_INFORMATION)?;
        let Some(wanted) = wanted else {
            if let Undo::Write(original) =
                self.eco
                    .undo(target, h.power_throttling(), Throttling::SYSTEM)
            {
                h.set_power_throttling(original)?;
            }
            return Ok(ExecOutcome::default());
        };
        self.eco
            .remember(target, || h.power_throttling(), Throttling::SYSTEM, alive);
        let written = h.set_power_throttling(wanted)?;
        self.eco.wrote(target, written);
        Ok(ExecOutcome::default())
    }

    fn set_cores(&self, target: ProcessKey, set: CoreSet) -> Result<ExecOutcome, ExecError> {
        if !self.topology.partitioned() {
            return Ok(ExecOutcome::default());
        }
        let h = Target::open(target, PROCESS_SET_LIMITED_INFORMATION)?;
        let Some(mut ids) = self.topology.ids_for(set) else {
            // 不限制：撤销调整。
            if let Undo::Write(original) = self.cores.undo(target, h.default_cpu_sets(), Vec::new())
            {
                h.set_default_cpu_sets(&original)?;
            }
            return Ok(ExecOutcome::default());
        };
        ids.sort_unstable();
        self.cores
            .remember(target, || h.default_cpu_sets(), Vec::new(), alive);
        h.set_default_cpu_sets(&ids)?;
        self.cores.wrote(target, ids);
        Ok(ExecOutcome::default())
    }

    /// 先记录启动信息，再结束进程，最后以原用户身份重新启动。
    fn restart_app(&self, target: ProcessKey) -> Result<ExecOutcome, ExecError> {
        let pid = target.pid;
        let spec = cmdline::capture(pid)?;
        let mut session_id = 0u32;
        // SAFETY: 输出到本地变量。
        unsafe { ProcessIdToSessionId(pid, &mut session_id) }.map_err(|e| map_error(&e))?;
        if session_id == 0 {
            return Err(ExecError::Failed("不重启会话 0 的进程".into()));
        }
        terminate(target)?;
        let new_pid = self.launcher.launch(&spec, session_id)?;
        Ok(ExecOutcome {
            new_pid: Some(new_pid),
        })
    }
}

/// 结束进程并等待它真正退出（最多 5 秒）。
fn terminate(target: ProcessKey) -> Result<(), ExecError> {
    let h = Target::open(target, PROCESS_TERMINATE | PROCESS_SYNCHRONIZE)?;
    // SAFETY: 句柄具有 PROCESS_TERMINATE 权限。
    unsafe { TerminateProcess(h.raw(), 1) }.map_err(|e| h.failure(&e))?;
    // SAFETY: 句柄具有 SYNCHRONIZE 权限。
    if unsafe { WaitForSingleObject(h.raw(), 5000) } != WAIT_OBJECT_0 {
        return Err(ExecError::Failed("进程没有在 5 秒内退出".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command};

    use poise_core::model::Pid;

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

    /// 测试里不核对创建时间。
    fn key(pid: Pid) -> ProcessKey {
        ProcessKey { pid, created: 0 }
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
            ex.execute(key(pid), &action)
                .unwrap_or_else(|e| panic!("{action:?}: {e}"));
        }
    }

    #[test]
    fn undoing_returns_what_the_program_had_unless_it_changed_it_since() {
        let child = Sandbox::spawn();
        let ex = executor();
        let pid = child.pid();
        let k = key(pid);
        let open = || Target::open(k, PROCESS_SET_INFORMATION).unwrap();
        let eco = |state| Action::SetEcoQos { pid, state };
        // 程序自己开了降频（不含计时器那一项）。
        let own = Throttling {
            control: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            state: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        open().set_power_throttling(own).unwrap();
        ex.execute(k, &eco(EcoQos::Off)).unwrap();
        assert_eq!(open().power_throttling().unwrap().state, 0);
        ex.execute(k, &eco(EcoQos::Default)).unwrap();
        assert_eq!(open().power_throttling(), Some(own));
        // 调整期间程序自己改了：撤销时不动。
        ex.execute(k, &eco(EcoQos::On)).unwrap();
        open().set_power_throttling(Throttling::SYSTEM).unwrap();
        ex.execute(k, &eco(EcoQos::Default)).unwrap();
        assert_eq!(open().power_throttling(), Some(Throttling::SYSTEM));
    }

    #[test]
    fn realtime_priority_is_refused() {
        let child = Sandbox::spawn();
        let r = executor().execute(
            key(child.pid()),
            &Action::SetPriority {
                pid: child.pid(),
                class: PriorityClass::Realtime,
            },
        );
        assert!(matches!(r, Err(ExecError::Failed(_))));
    }

    #[test]
    fn restart_app_relaunches_own_child() {
        let child = Sandbox::spawn();
        let out = executor()
            .execute(key(child.pid()), &Action::RestartApp { pid: child.pid() })
            .expect("restart");
        let new_pid = out.new_pid.expect("new pid");
        assert_ne!(new_pid, child.pid());
        // 清理重新启动出来的测试进程。
        let _ = terminate(key(new_pid));
    }

    #[test]
    fn missing_process_reports_gone() {
        let r = executor().execute(
            key(0xFFFF_FFF0),
            &Action::SetPriority {
                pid: 0xFFFF_FFF0,
                class: PriorityClass::Normal,
            },
        );
        assert_eq!(r, Err(ExecError::Gone));
    }
}
