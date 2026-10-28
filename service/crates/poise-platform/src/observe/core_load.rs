//! 大核 / 小核各自的负载：按逻辑处理器读取忙碌时间，与上一次相比得出占用比例。只读。

use std::ffi::c_void;

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};

use super::topology::CpuTopology;

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

/// 大核、小核的平均占用（0–100）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CoreLoad {
    pub performance_pct: f32,
    pub efficiency_pct: f32,
}

/// 每次调用 [`CoreLoadMeter::sample`] 得到与上一次调用之间的负载。不是大小核处理器时不工作。
#[derive(Default)]
pub struct CoreLoadMeter {
    performance_by_index: Vec<bool>,
    last: Vec<ProcessorTimes>,
}

impl CoreLoadMeter {
    pub fn new(topology: &CpuTopology) -> Self {
        Self {
            performance_by_index: topology.performance_by_index.clone(),
            last: Vec::new(),
        }
    }

    pub fn sample(&mut self) -> Option<CoreLoad> {
        if self.performance_by_index.is_empty() {
            return None;
        }
        let now = read_times(self.performance_by_index.len())?;
        let previous = std::mem::replace(&mut self.last, now);
        if previous.len() != self.last.len() {
            return None;
        }
        let mut busy = [0i64; 2];
        let mut total = [0i64; 2];
        for (i, (old, new)) in previous.iter().zip(&self.last).enumerate() {
            let class = usize::from(!self.performance_by_index[i]);
            let all = (new.kernel - old.kernel) + (new.user - old.user);
            let idle = new.idle - old.idle;
            total[class] += all.max(0);
            busy[class] += (all - idle).max(0);
        }
        let pct = |c: usize| {
            if total[c] == 0 {
                0.0
            } else {
                (busy[c] as f64 * 100.0 / total[c] as f64) as f32
            }
        };
        Some(CoreLoad {
            performance_pct: pct(0),
            efficiency_pct: pct(1),
        })
    }
}

fn read_times(count: usize) -> Option<Vec<ProcessorTimes>> {
    let mut buf = vec![ProcessorTimes::default(); count];
    let bytes = (count * std::mem::size_of::<ProcessorTimes>()) as u32;
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
    status.ok().ok()?;
    buf.truncate(written as usize / std::mem::size_of::<ProcessorTimes>());
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_system_structure() {
        assert_eq!(std::mem::size_of::<ProcessorTimes>(), 48);
    }

    #[test]
    fn measures_this_machine() {
        let mut m = CoreLoadMeter::new(&CpuTopology::detect());
        if m.performance_by_index.is_empty() {
            return;
        }
        assert!(m.sample().is_none());
        std::thread::sleep(std::time::Duration::from_millis(200));
        let load = m.sample().unwrap();
        assert!((0.0..=100.0).contains(&load.performance_pct));
        assert!((0.0..=100.0).contains(&load.efficiency_pct));
    }
}
