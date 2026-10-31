//! 系统级指标：CPU 时间、内存、提交量、内核内存池、实际频率、电源。

use std::ffi::c_void;
use std::sync::OnceLock;

use poise_core::model::{ProcessorTime, SystemMetrics};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Power::{
    CallNtPowerInformation, GetSystemPowerStatus, ProcessorInformation, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{
    GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetCurrentThread, GetSystemTimes, GetThreadTimes,
};

/// CallNtPowerInformation(ProcessorInformation) 返回的每核结构。
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessorPowerInformation {
    number: u32,
    max_mhz: u32,
    current_mhz: u32,
    mhz_limit: u32,
    max_idle_state: u32,
    current_idle_state: u32,
}

/// 当前线程累计用掉的处理器时间（内核 + 用户）。与经过的时间比较，可以分清一段工作是真的耗时，
/// 还是在等处理器（Poise 以低优先级运行，整机很忙时会排队）。
pub fn thread_cpu_time() -> std::time::Duration {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: 当前线程的伪句柄，输出到本地变量。
    if unsafe { GetThreadTimes(GetCurrentThread(), created, exited, kernel, user) }.is_err() {
        return std::time::Duration::ZERO;
    }
    let ticks = |t: &FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    std::time::Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

/// 全部处理器组的逻辑处理器数（`GetSystemInfo` 只数调用线程所在的组，超过 64 个逻辑处理器时偏少）。
/// 运行期间不变，只问一次。
pub fn logical_cpus() -> u32 {
    static COUNT: OnceLock<u32> = OnceLock::new();
    // SAFETY: 无参数。
    *COUNT.get_or_init(|| unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1))
}

/// 开机到现在的毫秒数。
pub fn uptime_ms() -> u64 {
    // SAFETY: 无参数。
    unsafe { GetTickCount64() }
}

fn filetime(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 整机指标。`processors` 是这一轮读到的各逻辑处理器的时间（见 `processors`）：只有一个处理器组时
/// 整机 CPU 时间由它们相加，不再单独查一次（系统查整机时间也是逐个处理器读出来再相加）。
pub fn read_metrics(processors: &[ProcessorTime]) -> SystemMetrics {
    let mut m = SystemMetrics {
        logical_cpus: logical_cpus(),
        uptime_ms: uptime_ms(),
        on_battery: on_battery(),
        ..Default::default()
    };
    if !processors.is_empty() && super::processors::single_group() {
        m.cpu_total_time = processors.iter().map(|p| p.total).sum();
        m.cpu_busy_time = processors.iter().map(|p| p.busy).sum();
    } else {
        read_cpu_times(&mut m);
    }
    read_memory(&mut m);
    read_frequency(&mut m);
    m
}

fn read_cpu_times(m: &mut SystemMetrics) {
    let (mut idle, mut kernel, mut user) = Default::default();
    // SAFETY: 输出到本地变量。
    if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok() {
        // 内核时间包含空闲时间。
        let total = filetime(kernel) + filetime(user);
        m.cpu_total_time = total;
        m.cpu_busy_time = total.saturating_sub(filetime(idle));
    }
}

fn read_memory(m: &mut SystemMetrics) {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: dwLength 已正确设置。
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
        m.mem_total = status.ullTotalPhys;
        m.mem_available = status.ullAvailPhys;
    }
    let mut perf = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: cb 已正确设置。
    if unsafe { GetPerformanceInfo(&mut perf, perf.cb) }.is_ok() {
        let page = perf.PageSize as u64;
        m.commit_total = perf.CommitTotal as u64 * page;
        m.commit_limit = perf.CommitLimit as u64 * page;
        m.kernel_nonpaged = perf.KernelNonpaged as u64 * page;
        m.kernel_paged = perf.KernelPaged as u64 * page;
        m.system_cache = perf.SystemCache as u64 * page;
        m.handle_count = perf.HandleCount;
        m.thread_count = perf.ThreadCount;
        m.process_count = perf.ProcessCount;
    }
}

fn read_frequency(m: &mut SystemMetrics) {
    let n = m.logical_cpus.max(1) as usize;
    let mut info = vec![ProcessorPowerInformation::default(); n];
    // SAFETY: 输出缓冲区大小为 n 个结构体。
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(info.as_mut_ptr() as *mut c_void),
            (n * size_of::<ProcessorPowerInformation>()) as u32,
        )
    };
    if status.is_ok() {
        m.cpu_mhz_current =
            (info.iter().map(|i| i.current_mhz as u64).sum::<u64>() / n as u64) as u32;
        m.cpu_mhz_base = info.iter().map(|i| i.max_mhz).max().unwrap_or(0);
    }
}

/// 是否在用电池。
pub fn on_battery() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    // SAFETY: 输出到本地结构体。
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.ACLineStatus == 0
}

/// 系统盘的剩余空间与总大小（字节）。
pub fn system_drive_space() -> Option<(u64, u64)> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    drive.chars().next().and_then(drive_space)
}

/// 某个盘符的剩余空间与总大小（字节）。
pub fn drive_space(letter: char) -> Option<(u64, u64)> {
    let root = crate::win::wide::WideString::new(&format!("{letter}:\\"));
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: 路径缓冲区在调用期间有效，输出到本地变量。
    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            root.as_pcwstr(),
            None,
            Some(&mut total),
            Some(&mut free),
        )
    }
    .ok()?;
    Some((free, total))
}

/// 当前物理控制台的会话 ID（界面代理尚未上报时的默认值）。
pub fn active_console_session() -> u32 {
    // SAFETY: 无参数。
    unsafe { windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() }
}

/// 进程所在的会话；进程已退出时为 None。
pub fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: 输出到本地变量。
    unsafe { windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(pid, &mut session) }
        .ok()?;
    Some(session)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_plausible_metrics() {
        let m = super::read_metrics(&crate::observe::processors::read_processor_times());
        assert!(m.logical_cpus >= 1);
        assert!(m.mem_total > m.mem_available);
        assert!(m.commit_limit >= m.commit_total);
        assert!(m.cpu_total_time >= m.cpu_busy_time);
        assert!(m.cpu_mhz_base > 0);
    }
}
