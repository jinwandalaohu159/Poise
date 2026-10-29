//! Poise 让自己保持轻量：运行在 E 核、开启效率模式（降频、放宽计时器精度）、低内存优先级、低于正常的 CPU 优先级。
//! 服务和托盘代理启动时都调用：任何程序和 Poise 抢资源，都是程序优先。
//!
//! 例外是关键路径上的少数线程（[`responsive_thread`]）：每轮采样与调度、界面的请求。它们每轮只用几毫秒，
//! 但如果和被让路的后台程序一起在小核上排队，实测一轮要等 0.1～9 秒才轮到（自身只要 6～35 毫秒），
//! 前台切换、让路与恢复都跟着变慢。所以这几个线程以正常优先级、不降频运行——用的处理器时间不变，只是不再排队。

use std::ffi::c_void;

use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetCurrentThread, MEMORY_PRIORITY_INFORMATION,
    MEMORY_PRIORITY_LOW, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, ProcessMemoryPriority, ProcessPowerThrottling,
    SetPriorityClass, SetProcessDefaultCpuSets, SetProcessInformation, SetThreadInformation,
    SetThreadPriority, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_HIGHEST, ThreadPowerThrottling,
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

/// 让当前线程及时运行（见模块说明）：在"低于正常"的进程里取最高的线程优先级（相当于普通程序的正常优先级），
/// 并且这个线程不受效率模式降频。仍然只在小核上运行。
pub fn responsive_thread() {
    // SAFETY: 只作用于当前线程，参数为本地结构体。
    unsafe {
        let me = GetCurrentThread();
        let _ = SetThreadPriority(me, THREAD_PRIORITY_HIGHEST);
        let state = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        let _ = SetThreadInformation(
            me,
            ThreadPowerThrottling,
            &state as *const _ as *const c_void,
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Threading::GetThreadPriority;

    use super::*;

    #[test]
    fn responsive_thread_raises_only_the_calling_thread() {
        let raised = std::thread::spawn(|| {
            responsive_thread();
            // SAFETY: 读取当前线程的优先级。
            unsafe { GetThreadPriority(GetCurrentThread()) }
        })
        .join()
        .unwrap();
        assert_eq!(raised, THREAD_PRIORITY_HIGHEST.0);
        // SAFETY: 同上。
        assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, 0);
    }
}
