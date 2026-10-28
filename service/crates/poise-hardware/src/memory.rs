//! 通用层：内存规格（总量、频率、每条的容量）。只在启动时读一次，只读。

use poise_platform::win::wmi::WmiNamespace;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub speed_mhz: u32,
    /// 每条内存的容量。
    pub module_bytes: Vec<u64>,
}

/// 内存总量、频率、每条的容量。
pub fn read_memory() -> Option<MemoryInfo> {
    let ns = WmiNamespace::connect(r"root\cimv2").ok()?;
    let modules = ns
        .query("SELECT Capacity, ConfiguredClockSpeed FROM Win32_PhysicalMemory")
        .ok()?;
    let module_bytes: Vec<u64> = modules.iter().filter_map(|m| m.u64("Capacity")).collect();
    (!modules.is_empty()).then(|| MemoryInfo {
        total_bytes: module_bytes.iter().sum(),
        speed_mhz: modules
            .iter()
            .filter_map(|m| m.u64("ConfiguredClockSpeed"))
            .min()
            .unwrap_or(0) as u32,
        module_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_machines_memory() {
        let mem = read_memory().unwrap();
        assert!(mem.total_bytes > 0 && !mem.module_bytes.is_empty());
    }
}
