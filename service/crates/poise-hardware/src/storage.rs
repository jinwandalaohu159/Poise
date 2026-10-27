//! 通用层：硬盘（温度、健康、磨损）、系统盘空间、内存规格。任何电脑都能用，只读。
//!
//! 硬盘数据来自系统存储管理（需要管理员权限，服务以 SYSTEM 运行时可用），变化很慢，
//! 每 5 分钟刷新一次；内存规格只在启动时读一次。

use poise_core::model::Millis;
use poise_platform::observe::system;
use poise_platform::win::wmi::WmiNamespace;

const REFRESH_EVERY_MS: Millis = 5 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskHealth {
    Healthy,
    Warning,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReading {
    pub name: String,
    pub temp_c: Option<u32>,
    pub health: DiskHealth,
    /// 寿命已用百分比（固态硬盘）。
    pub wear_pct: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub speed_mhz: u32,
    pub modules: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageReading {
    pub disks: Vec<DiskReading>,
    /// 系统盘（剩余，总大小）。
    pub system_drive: Option<(u64, u64)>,
}

#[derive(Default)]
pub struct StorageMonitor {
    last_at: Option<Millis>,
    latest: StorageReading,
}

impl StorageMonitor {
    /// 到时间才重新读取，否则返回上次的结果。
    pub fn read(&mut self, now: Millis) -> &StorageReading {
        let due = self
            .last_at
            .is_none_or(|t| now.saturating_sub(t) >= REFRESH_EVERY_MS);
        if due {
            self.last_at = Some(now);
            self.latest = StorageReading {
                disks: read_disks(),
                system_drive: system::system_drive_space(),
            };
        }
        &self.latest
    }
}

fn read_disks() -> Vec<DiskReading> {
    let ns = match WmiNamespace::connect(r"root\Microsoft\Windows\Storage") {
        Ok(ns) => ns,
        Err(e) => {
            log::debug!("读取硬盘信息失败：{e}");
            return Vec::new();
        }
    };
    let disks = ns
        .query("SELECT DeviceId, FriendlyName, HealthStatus FROM MSFT_PhysicalDisk")
        .unwrap_or_default();
    // 可靠性计数器（温度、磨损）需要管理员权限，读不到时只显示名称和健康状态。
    let counters = ns
        .query("SELECT DeviceId, Temperature, Wear FROM MSFT_StorageReliabilityCounter")
        .unwrap_or_default();
    disks
        .iter()
        .map(|d| {
            let id = d.string("DeviceId");
            let counter = counters.iter().find(|c| c.string("DeviceId") == id);
            DiskReading {
                name: d
                    .string("FriendlyName")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                temp_c: counter
                    .and_then(|c| c.u64("Temperature"))
                    .filter(|t| (1..=125).contains(t))
                    .map(|t| t as u32),
                health: match d.u64("HealthStatus") {
                    Some(0) => DiskHealth::Healthy,
                    Some(1) => DiskHealth::Warning,
                    Some(2) => DiskHealth::Unhealthy,
                    _ => DiskHealth::Unknown,
                },
                wear_pct: counter.and_then(|c| c.u64("Wear")).map(|w| w as u32),
            }
        })
        .collect()
}

/// 内存总量、频率、条数。
pub fn read_memory() -> Option<MemoryInfo> {
    let ns = WmiNamespace::connect(r"root\cimv2").ok()?;
    let modules = ns
        .query("SELECT Capacity, ConfiguredClockSpeed FROM Win32_PhysicalMemory")
        .ok()?;
    (!modules.is_empty()).then(|| MemoryInfo {
        total_bytes: modules.iter().filter_map(|m| m.u64("Capacity")).sum(),
        speed_mhz: modules
            .iter()
            .filter_map(|m| m.u64("ConfiguredClockSpeed"))
            .min()
            .unwrap_or(0) as u32,
        modules: modules.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_machines_memory_and_disks() {
        let mem = read_memory().unwrap();
        assert!(mem.total_bytes > 0 && mem.modules > 0);
        let mut monitor = StorageMonitor::default();
        let r = monitor.read(0).clone();
        assert!(r.system_drive.is_some_and(|(free, total)| free <= total));
        // 名称和健康状态不需要管理员权限。
        assert!(r.disks.iter().any(|d| !d.name.is_empty()));
    }
}
