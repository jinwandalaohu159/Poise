//! CPU 拓扑：通过 EfficiencyClass 自动识别 P 核与 E 核，不写死核心编号。

use poise_core::action::CoreSet;
use windows::Win32::System::SystemInformation::{
    CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
};
use windows::Win32::System::Threading::GetCurrentProcess;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuTopology {
    pub performance: Vec<u32>,
    pub efficiency: Vec<u32>,
}

impl CpuTopology {
    pub fn detect() -> Self {
        let sets = read_cpu_sets();
        let (Some(max), Some(min)) = (
            sets.iter().map(|s| s.1).max(),
            sets.iter().map(|s| s.1).min(),
        ) else {
            return Self::default();
        };
        if max == min {
            // 不是大小核架构：两个集合都为空，限制核心的动作将不生效。
            return Self::default();
        }
        Self {
            performance: sets.iter().filter(|s| s.1 == max).map(|s| s.0).collect(),
            efficiency: sets.iter().filter(|s| s.1 == min).map(|s| s.0).collect(),
        }
    }

    pub fn is_hybrid(&self) -> bool {
        !self.performance.is_empty() && !self.efficiency.is_empty()
    }

    /// 某个核心集合对应的 CPU Set ID；None 表示不限制。
    pub fn ids_for(&self, set: CoreSet) -> Option<&[u32]> {
        if !self.is_hybrid() {
            return None;
        }
        match set {
            CoreSet::All => None,
            CoreSet::Performance => Some(&self.performance),
            CoreSet::Efficiency => Some(&self.efficiency),
        }
    }
}

/// 返回 (CPU Set ID, EfficiencyClass)。
fn read_cpu_sets() -> Vec<(u32, u8)> {
    let mut len = 0u32;
    // SAFETY: 先询问所需大小。
    let _ =
        unsafe { GetSystemCpuSetInformation(None, 0, &mut len, Some(GetCurrentProcess()), None) };
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节。
    let ok = unsafe {
        GetSystemCpuSetInformation(
            Some(buf.as_mut_ptr() as *mut SYSTEM_CPU_SET_INFORMATION),
            len,
            &mut len,
            Some(GetCurrentProcess()),
            None,
        )
    };
    if !ok.as_bool() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let base = buf.as_ptr() as *const u8;
    let mut offset = 0usize;
    while offset < len as usize {
        // SAFETY: 每个条目以 Size 字段给出自身长度，始终在 len 范围内。
        let entry = unsafe { &*(base.add(offset) as *const SYSTEM_CPU_SET_INFORMATION) };
        if entry.Type == CpuSetInformation {
            // SAFETY: Type 为 CpuSetInformation 时联合体的 CpuSet 成员有效。
            let cs = unsafe { entry.Anonymous.CpuSet };
            out.push((cs.Id, cs.EfficiencyClass));
        }
        if entry.Size == 0 {
            break;
        }
        offset += entry.Size as usize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hybrid_layout_on_this_machine() {
        let t = CpuTopology::detect();
        // 开发机为 i7-12700H（6P+8E，20 线程）；其他机器上至少不应崩溃。
        if t.is_hybrid() {
            assert_eq!(
                t.performance.len() + t.efficiency.len(),
                crate::observe::system::logical_cpus() as usize
            );
            assert!(t.ids_for(CoreSet::All).is_none());
        }
    }
}
