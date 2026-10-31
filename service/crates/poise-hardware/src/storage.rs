//! 通用层：硬盘（容量、温度、健康、磨损、各分区空间）、系统盘空间。任何电脑都能用，只读。
//!
//! 硬盘数据来自系统存储管理（需要管理员权限，服务以 SYSTEM 运行时可用），变化很慢：
//! 界面显示着时每 5 分钟刷新一次；后台只用于健康提示，半小时一次，并且不读温度和磨损——
//! 读它们要给每块硬盘发健康查询命令，会唤醒休眠中的机械硬盘和移动硬盘。

use poise_core::model::Millis;
use poise_platform::observe::system;
use poise_platform::win::wmi::{WmiNamespace, WmiObject};

/// 界面显示着时多久刷新一次。
const SHOWN_EVERY_MS: Millis = 5 * 60_000;
/// 后台多久刷新一次。
const BACKGROUND_EVERY_MS: Millis = 30 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskHealth {
    Healthy,
    Warning,
    Unhealthy,
    Unknown,
}

/// 硬盘上的一个分区（有盘符的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeReading {
    pub letter: char,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReading {
    pub name: String,
    /// 整块硬盘的容量。
    pub size_bytes: Option<u64>,
    pub temp_c: Option<u32>,
    pub health: DiskHealth,
    /// 寿命已用百分比（固态硬盘）。
    pub wear_pct: Option<u32>,
    /// 这块硬盘上有盘符的分区，按盘符排序。
    pub volumes: Vec<VolumeReading>,
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
    /// 上次读了温度和磨损（界面显示着时读的）。
    counters: bool,
}

impl StorageMonitor {
    /// 上次的结果（不读取）。
    pub fn latest(&self) -> &StorageReading {
        &self.latest
    }

    /// 到时间才重新读取，否则返回上次的结果。`shown`：界面显示着（刷新得勤一些）。
    pub fn read(&mut self, now: Millis, shown: bool) -> &StorageReading {
        let every = if shown {
            SHOWN_EVERY_MS
        } else {
            BACKGROUND_EVERY_MS
        };
        // 界面刚打开、上次是后台读的（没有温度和磨损）：立即补读。
        let due = self.last_at.is_none_or(|t| now.saturating_sub(t) >= every)
            || (shown && !self.counters);
        if due {
            self.last_at = Some(now);
            self.counters = shown;
            self.latest = StorageReading {
                disks: read_disks(shown),
                system_drive: system::system_drive_space(),
            };
        }
        &self.latest
    }
}

/// `counters`：也读温度和磨损（见模块说明）。
fn read_disks(counters: bool) -> Vec<DiskReading> {
    let ns = match WmiNamespace::connect(r"root\Microsoft\Windows\Storage") {
        Ok(ns) => ns,
        Err(e) => {
            log::debug!("读取硬盘信息失败：{e}");
            return Vec::new();
        }
    };
    let disks = ns
        .query("SELECT DeviceId, FriendlyName, HealthStatus, Size FROM MSFT_PhysicalDisk")
        .unwrap_or_default();
    // 分区与硬盘的对应：MSFT_Partition.DiskNumber 就是 MSFT_PhysicalDisk.DeviceId。
    let partitions = ns
        .query("SELECT DiskNumber, DriveLetter FROM MSFT_Partition")
        .unwrap_or_default();
    // 可靠性计数器（温度、磨损）需要管理员权限，读不到时只显示名称和健康状态。
    let counters = if counters {
        ns.query("SELECT DeviceId, Temperature, Wear FROM MSFT_StorageReliabilityCounter")
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    disks
        .iter()
        .map(|d| {
            let id = d.string("DeviceId");
            let counter = counters.iter().find(|c| c.string("DeviceId") == id);
            DiskReading {
                // 型号里常有连续空格（"WD_BLACK  SN750"），合并成一个。
                name: d
                    .string("FriendlyName")
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                size_bytes: d.u64("Size").filter(|&s| s > 0),
                temp_c: counter
                    .and_then(|c| c.u64("Temperature"))
                    .filter(|&t| crate::plausible_celsius(t))
                    .map(|t| t as u32),
                health: match d.u64("HealthStatus") {
                    Some(0) => DiskHealth::Healthy,
                    Some(1) => DiskHealth::Warning,
                    Some(2) => DiskHealth::Unhealthy,
                    _ => DiskHealth::Unknown,
                },
                wear_pct: counter.and_then(|c| c.u64("Wear")).map(|w| w as u32),
                volumes: volumes_on(&partitions, id.as_deref()),
            }
        })
        .collect()
}

/// 某块硬盘上有盘符的分区及其空间。
fn volumes_on(partitions: &[WmiObject], disk_id: Option<&str>) -> Vec<VolumeReading> {
    let Some(disk) = disk_id.and_then(|id| id.trim().parse::<u64>().ok()) else {
        return Vec::new();
    };
    let mut volumes: Vec<VolumeReading> = partitions
        .iter()
        .filter(|p| p.u64("DiskNumber") == Some(disk))
        .filter_map(|p| char::from_u32(p.u64("DriveLetter")? as u32))
        .filter(char::is_ascii_alphabetic)
        .filter_map(|letter| {
            let (free_bytes, total_bytes) = system::drive_space(letter)?;
            Some(VolumeReading {
                letter: letter.to_ascii_uppercase(),
                free_bytes,
                total_bytes,
            })
        })
        .collect();
    volumes.sort_by_key(|v| v.letter);
    volumes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_machines_disks() {
        let mut monitor = StorageMonitor::default();
        let r = monitor.read(0, true).clone();
        assert!(r.system_drive.is_some_and(|(free, total)| free <= total));
        // 名称和健康状态不需要管理员权限。
        assert!(r.disks.iter().any(|d| !d.name.is_empty()));
        // 系统盘所在的硬盘能列出 C 盘。
        assert!(
            r.disks
                .iter()
                .flat_map(|d| &d.volumes)
                .any(|v| v.letter == 'C')
        );
    }
}
