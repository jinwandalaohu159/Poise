//! 单个进程的显卡 3D 引擎占用（任务管理器"GPU"列的同一来源）。一次只测一个进程（前台全屏程序），
//! 用来判断它是不是正在渲染画面的游戏。
//!
//! 优先直接问显卡内核（[`kmt`]，开始测约 0.2 毫秒，每次读数几十微秒）；拿不到完整查询权限的进程句柄
//! （受保护的进程）或者系统不支持时，退回性能计数器（[`pdh`]，开始测一次约 0.5 秒处理器时间）。换测别的进程时，刚测过的那个留着：
//! 前台在游戏和别的窗口之间来回切换很常见。
//!
//! [`DiscreteActivity`]：独立显卡有没有在干活（读 NVML 之前看一眼，不叫醒休眠中的独显）。

mod awake;
mod kmt;
mod pdh;

pub use awake::DiscreteActivity;

use std::time::Instant;

use poise_core::model::Pid;
use windows::Win32::System::Threading::PROCESS_QUERY_INFORMATION;

use crate::win::handle::{OwnedHandle, open_process};
use kmt::KernelStats;

#[derive(Default)]
pub struct GpuMeter {
    /// 显卡和 3D 引擎（每开始测一个新进程时重新枚举，显卡变了也能跟上）。
    kernel: Option<KernelStats>,
    current: Option<Probe>,
    previous: Option<Probe>,
}

impl GpuMeter {
    /// 进程在上次调用以来的 3D 引擎占用（0～100，多个显卡引擎相加）。
    /// 刚开始测某个进程时还没有可比较的两次采样，返回 None；进程没用显卡时返回 0。
    pub fn usage_3d(&mut self, pid: Pid) -> Option<f32> {
        if self.current.as_ref().map(|p| p.pid) != Some(pid) {
            let reused = self.previous.take().filter(|p| p.pid == pid);
            self.previous = self.current.take();
            self.current = match reused {
                // 留着的测量上次读数已经是很久以前：重新起算，下次读到的才是此刻的占用。
                Some(mut probe) => {
                    probe.rebase(self.kernel.as_ref());
                    Some(probe)
                }
                None => self.open(pid),
            };
            return None;
        }
        let probe = self.current.as_mut()?;
        let reading = probe.read(self.kernel.as_ref());
        if reading.is_none() && matches!(probe.source, Source::Kernel { .. }) {
            // 显卡变了（驱动重装、外接显卡）：下次重新枚举、重新开始测。
            self.kernel = None;
            self.current = None;
        }
        reading
    }

    /// 不再需要测量时释放。
    pub fn stop(&mut self) {
        self.current = None;
        self.previous = None;
    }

    fn open(&mut self, pid: Pid) -> Option<Probe> {
        self.kernel = KernelStats::open();
        let kernel = self.kernel.as_ref().and_then(|k| {
            // 要完整的查询权限，见 `KernelStats::running_3d`。
            let handle = open_process(pid, PROCESS_QUERY_INFORMATION).ok()?;
            let running = k.first_running_3d(handle.raw())?;
            Some(Source::Kernel {
                handle,
                last: (Instant::now(), running),
            })
        });
        let source = match kernel {
            Some(s) => s,
            None => Source::Counters(pdh::Query::open(pid)?),
        };
        Some(Probe { pid, source })
    }
}

/// 对一个进程的测量。
struct Probe {
    pid: Pid,
    source: Source,
}

enum Source {
    /// 直接问显卡内核：进程句柄，上次读数的时刻与累计运行时间（100 纳秒）。
    Kernel {
        handle: OwnedHandle,
        last: (Instant, u64),
    },
    /// 性能计数器。
    Counters(pdh::Query),
}

impl Probe {
    fn read(&mut self, kernel: Option<&KernelStats>) -> Option<f32> {
        match &mut self.source {
            Source::Kernel { handle, last } => {
                let running = kernel?.running_3d(handle.raw())?;
                let now = Instant::now();
                let wall_100ns = now.duration_since(last.0).as_nanos() as f64 / 100.0;
                let busy = running.saturating_sub(last.1) as f64;
                *last = (now, running);
                (wall_100ns > 0.0).then(|| (busy / wall_100ns * 100.0).clamp(0.0, 100.0) as f32)
            }
            Source::Counters(q) => q.read(),
        }
    }

    /// 重新起算：下次读到的是从现在开始的占用。
    fn rebase(&mut self, kernel: Option<&KernelStats>) {
        match &mut self.source {
            Source::Kernel { handle, last } => {
                if let Some(running) = kernel.and_then(|k| k.running_3d(handle.raw())) {
                    *last = (Instant::now(), running);
                }
            }
            Source::Counters(q) => q.collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_without_rendering_reads_zero_after_the_first_sample() {
        let mut meter = GpuMeter::default();
        let pid = std::process::id();
        assert_eq!(meter.usage_3d(pid), None);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(meter.usage_3d(pid), Some(0.0));
        // 换测别的进程再换回来：用留着的测量，重新起算。
        assert_eq!(meter.usage_3d(4), None);
        assert_eq!(meter.usage_3d(pid), None);
        assert!(meter.previous.as_ref().is_some_and(|p| p.pid == 4));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(meter.usage_3d(pid), Some(0.0));
    }

    #[test]
    fn a_process_the_kernel_says_nothing_about_is_not_measured_through_it() {
        // 没用过显卡的进程（测试进程自己）：每个引擎都回答"参数错误"。读数照常是 0，但开始测时不走
        // 显卡内核这条路（分不清是没用过还是系统不肯回答，见 `KernelStats::first_running_3d`）。
        let Some(kernel) = KernelStats::open() else {
            return;
        };
        let handle = open_process(std::process::id(), PROCESS_QUERY_INFORMATION).unwrap();
        assert_eq!(kernel.running_3d(handle.raw()), Some(0));
        assert_eq!(kernel.first_running_3d(handle.raw()), None);
    }
}
