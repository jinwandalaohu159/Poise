//! 要调整的进程：打开时核对身份（进程号会被新进程复用），设置失败时分辨是进程已经退出还是真的失败；
//! 效率模式（电源节流）的标志按系统支持的取舍；内存优先级的读写。调整别的程序和 Poise 调整自己共用。

use std::ffi::c_void;

use poise_core::action::ExecError;
use poise_core::model::ProcessKey;
use windows::Win32::Foundation::{FILETIME, HANDLE, STILL_ACTIVE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessDefaultCpuSets, GetProcessInformation, GetProcessTimes,
    MEMORY_PRIORITY, MEMORY_PRIORITY_INFORMATION, PROCESS_ACCESS_RIGHTS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    ProcessMemoryPriority, ProcessPowerThrottling, SetProcessDefaultCpuSets, SetProcessInformation,
    WaitForSingleObject,
};

use super::handle::{OwnedHandle, map_error, open_process};

/// 进程在 `within` 之内退出了（已经不在也算）。打不开（没有权限等）时按没退出。
pub fn exits_within(pid: u32, within: std::time::Duration) -> bool {
    match open_process(pid, PROCESS_SYNCHRONIZE) {
        Ok(h) => {
            let ms = u32::try_from(within.as_millis()).unwrap_or(u32::MAX);
            // SAFETY: 句柄有等待权限。
            let waited = unsafe { WaitForSingleObject(h.raw(), ms) };
            waited == WAIT_OBJECT_0
        }
        Err(ExecError::Gone) => true,
        Err(_) => false,
    }
}

/// 进程的创建时间（FILETIME，100ns）；打不开、已退出时为 None。和进程号一起认出"还是不是那个进程"。
pub fn created(pid: u32) -> Option<u64> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    creation_time(&h)
}

/// 打开的目标进程。
pub struct Target(OwnedHandle);

impl Target {
    /// 以 `access`（另加查询权限，用来核对身份）打开进程。`key.created` 为 0 时不核对。
    /// 创建时间对不上说明进程号已被新进程复用，按"已退出"处理；读不到创建时间就核对不了，按"拒绝访问"
    /// 处理（不当成已退出——那样会丢掉记下的原值、每轮再试）。都绝不动到别的程序。
    pub fn open(key: ProcessKey, access: PROCESS_ACCESS_RIGHTS) -> Result<Self, ExecError> {
        let h = open_process(key.pid, access | PROCESS_QUERY_LIMITED_INFORMATION)?;
        if key.created != 0 {
            match creation_time(&h) {
                Some(created) if created == key.created => {}
                Some(_) => return Err(ExecError::Gone),
                None => return Err(ExecError::AccessDenied),
            }
        }
        Ok(Self(h))
    }

    pub fn raw(&self) -> HANDLE {
        self.0.raw()
    }

    /// 设置失败的原因：进程已经退出时是"已退出"（有的设置此时报"参数错误"），否则按错误码。
    pub fn failure(&self, e: &windows::core::Error) -> ExecError {
        if self.exited() {
            ExecError::Gone
        } else {
            map_error(e)
        }
    }

    fn exited(&self) -> bool {
        let mut code = 0u32;
        // SAFETY: 句柄有查询权限，输出到本地变量。
        unsafe { GetExitCodeProcess(self.raw(), &mut code) }.is_ok()
            && code != STILL_ACTIVE.0 as u32
    }

    /// 设置效率模式（电源节流）。系统不认识"忽略计时器精度"（Windows 10）时去掉这一项，其余照设。
    /// 返回实际写入的值。
    pub fn set_power_throttling(&self, value: Throttling) -> Result<Throttling, ExecError> {
        set_power_throttling(self.raw(), value.control, value.state).map_err(|e| self.failure(&e))
    }

    /// 此刻的效率模式设置（程序自己或别人明确设过的项）；读不到时为 None。
    pub fn power_throttling(&self) -> Option<Throttling> {
        let mut info = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ..Default::default()
        };
        // SAFETY: 结构体大小与传入长度一致，版本号已填好。
        unsafe {
            GetProcessInformation(
                self.raw(),
                ProcessPowerThrottling,
                &mut info as *mut _ as *mut c_void,
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        }
        .ok()
        .map(|()| Throttling {
            control: info.ControlMask,
            state: info.StateMask,
        })
    }

    /// 进程的默认核心范围（CPU Set ID，排好序；空表示不限制）；读不到时为 None。
    pub fn default_cpu_sets(&self) -> Option<Vec<u32>> {
        let mut ids = [0u32; 64];
        let mut count = 0u32;
        // SAFETY: 缓冲区长度如实传入。
        let ok = unsafe { GetProcessDefaultCpuSets(self.raw(), Some(&mut ids), &mut count) };
        let mut ids = if ok.as_bool() {
            ids[..count as usize].to_vec()
        } else if count as usize > ids.len() {
            let mut more = vec![0u32; count as usize];
            // SAFETY: 缓冲区按系统要求的个数分配。
            let ok = unsafe { GetProcessDefaultCpuSets(self.raw(), Some(&mut more), &mut count) };
            if !ok.as_bool() {
                return None;
            }
            more.truncate(count as usize);
            more
        } else {
            return None;
        };
        ids.sort_unstable();
        Some(ids)
    }

    /// 设置进程的默认核心范围（空表示不限制）。
    pub fn set_default_cpu_sets(&self, ids: &[u32]) -> Result<(), ExecError> {
        let ids = (!ids.is_empty()).then_some(ids);
        // SAFETY: ids 为 None 时清除默认核心范围（即不限制）。
        let ok = unsafe { SetProcessDefaultCpuSets(self.raw(), ids) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(self.failure(&windows::core::Error::from_thread()))
        }
    }
}

/// 效率模式（电源节流）设置：`control` 是明确设置了的项，`state` 是其中开启的项；都为 0 表示交给系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Throttling {
    pub control: u32,
    pub state: u32,
}

impl Throttling {
    /// 交给系统决定。
    pub const SYSTEM: Self = Self {
        control: 0,
        state: 0,
    };
}

/// 设置进程的效率模式（电源节流），见 [`Target::set_power_throttling`]。返回实际写入的值。
pub fn set_power_throttling(
    h: HANDLE,
    control: u32,
    state: u32,
) -> windows::core::Result<Throttling> {
    let supported = if timer_flag_supported() {
        u32::MAX
    } else {
        !PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
    };
    let info = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control & supported,
        StateMask: state & supported,
    };
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        SetProcessInformation(
            h,
            ProcessPowerThrottling,
            &info as *const _ as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }?;
    Ok(Throttling {
        control: info.ControlMask,
        state: info.StateMask,
    })
}

/// 进程的内存优先级（需要查询权限）。
pub fn memory_priority(h: HANDLE) -> Option<MEMORY_PRIORITY> {
    let mut info = MEMORY_PRIORITY_INFORMATION::default();
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        GetProcessInformation(
            h,
            ProcessMemoryPriority,
            &mut info as *mut _ as *mut c_void,
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
    .ok()
    .map(|()| info.MemoryPriority)
}

/// 设置进程的内存优先级（需要设置权限）。
pub fn set_memory_priority(h: HANDLE, priority: MEMORY_PRIORITY) -> windows::core::Result<()> {
    let info = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: priority,
    };
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        SetProcessInformation(
            h,
            ProcessMemoryPriority,
            &info as *const _ as *const c_void,
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
}

/// 系统是否支持"忽略计时器精度"这一项（Windows 11 与 Server 2022 起）。不支持的系统会拒绝整个设置。
fn timer_flag_supported() -> bool {
    /// Windows Server 2022 的版本号；Windows 11 从 22000 开始，Windows 10 最高 19045。
    const FIRST_BUILD: u32 = 20348;
    // 读不到时按新系统处理（Poise 主要面向 Windows 11）。
    super::version::build().is_none_or(|b| b >= FIRST_BUILD)
}

/// 进程的创建时间（FILETIME，100ns），与进程表里的一致。
fn creation_time(h: &OwnedHandle) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: 句柄有查询权限，输出到本地变量。
    unsafe { GetProcessTimes(h.raw(), &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    Some(filetime(created))
}

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Threading::PROCESS_SET_INFORMATION;

    use super::*;

    #[test]
    fn a_reused_process_number_is_treated_as_gone() {
        let me = std::process::id();
        let h = open_process(me, PROCESS_QUERY_LIMITED_INFORMATION).unwrap();
        let created = creation_time(&h).unwrap();
        let key = |created| ProcessKey { pid: me, created };
        assert!(Target::open(key(created), PROCESS_SET_INFORMATION).is_ok());
        assert!(Target::open(key(0), PROCESS_SET_INFORMATION).is_ok());
        assert!(matches!(
            Target::open(key(created + 1), PROCESS_SET_INFORMATION),
            Err(ExecError::Gone)
        ));
    }
}
