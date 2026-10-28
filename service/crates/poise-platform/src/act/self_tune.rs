//! Poise 让自己保持轻量：运行在 E 核、开启效率模式（降频、放宽计时器精度）、低内存优先级、低于正常的 CPU 优先级。
//! 服务和托盘代理启动时都调用：任何程序和 Poise 抢资源，都是程序优先。

use std::ffi::c_void;

use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, MEMORY_PRIORITY_INFORMATION,
    MEMORY_PRIORITY_LOW, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, ProcessMemoryPriority, ProcessPowerThrottling,
    SetPriorityClass, SetProcessDefaultCpuSets, SetProcessInformation,
};

use crate::observe::topology::CpuTopology;

pub fn apply(topology: &CpuTopology) {
    // SAFETY: 所有调用都只作用于当前进程，参数为本地结构体。
    unsafe {
        let me = GetCurrentProcess();
        let _ = SetPriorityClass(me, BELOW_NORMAL_PRIORITY_CLASS);
        let throttle = PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;
        let eco = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: throttle,
            StateMask: throttle,
        };
        let _ = SetProcessInformation(
            me,
            ProcessPowerThrottling,
            &eco as *const _ as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        let mem = MEMORY_PRIORITY_INFORMATION {
            MemoryPriority: MEMORY_PRIORITY_LOW,
        };
        let _ = SetProcessInformation(
            me,
            ProcessMemoryPriority,
            &mem as *const _ as *const c_void,
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        );
        if let Some(ids) = topology.ids_for(poise_core::action::CoreSet::Efficiency) {
            let _ = SetProcessDefaultCpuSets(me, Some(ids));
        }
    }
}
