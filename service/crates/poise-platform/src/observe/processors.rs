//! 各逻辑处理器累计的忙碌时间（第 0 个处理器组，64 个逻辑处理器以内的电脑就是全部）。每轮采样读一次：
//! 整机 CPU 时间由它们相加（见 `system::read_metrics`），各核心的占用由 `ActivityTracker` 换算。只读。

use std::ffi::c_void;

use poise_core::model::ProcessorTime;
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
use windows::Win32::System::Threading::{GetActiveProcessorCount, GetActiveProcessorGroupCount};

const PROCESSOR_PERFORMANCE_CLASS: SYSTEM_INFORMATION_CLASS = SYSTEM_INFORMATION_CLASS(8);

/// SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION（x64）。内核时间包含空闲时间。
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessorTimes {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
}

/// 第 0 个处理器组各逻辑处理器的累计时间（按组内序号）；读不到时为空。
pub fn read_processor_times() -> Vec<ProcessorTime> {
    // SAFETY: 无参数。
    let count = unsafe { GetActiveProcessorCount(0) } as usize;
    if count == 0 {
        return Vec::new();
    }
    let mut buf = vec![ProcessorTimes::default(); count];
    let bytes = (count * size_of::<ProcessorTimes>()) as u32;
    let mut written = 0u32;
    // SAFETY: 缓冲区正好容纳 count 个结构，系统只写入不超过 bytes 的数据。
    let status = unsafe {
        NtQuerySystemInformation(
            PROCESSOR_PERFORMANCE_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            bytes,
            &mut written,
        )
    };
    if status.is_err() {
        return Vec::new();
    }
    buf.truncate(written as usize / size_of::<ProcessorTimes>());
    buf.iter()
        .map(|t| {
            let total = (t.kernel + t.user).max(0) as u64;
            ProcessorTime {
                busy: total.saturating_sub(t.idle.max(0) as u64),
                total,
            }
        })
        .collect()
}

/// 只有一个处理器组：上面读到的就是全部逻辑处理器，整机时间直接由它们相加。
pub fn single_group() -> bool {
    // SAFETY: 无参数。
    unsafe { GetActiveProcessorGroupCount() <= 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_system_structure() {
        assert_eq!(size_of::<ProcessorTimes>(), 48);
    }

    #[test]
    fn reads_every_processor_of_this_machine() {
        let times = read_processor_times();
        assert_eq!(times.len(), crate::observe::system::logical_cpus() as usize);
        assert!(times.iter().all(|t| t.busy <= t.total && t.total > 0));
    }
}
