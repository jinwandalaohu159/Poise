//! 硬件适配。
//!
//! - 通用层：电源计划参数（[`power`]）、NVIDIA 显卡读数（[`nvml`]）、硬盘与内存（[`storage`]），任何电脑都能用；
//! - 机型层：[`profiles`]，每台电脑一份档案，提供散热模式、温度、风扇等机型特有能力。
//!
//! 边界：只用官方接口与已验证的取值；不超频、不降压、不写 CPU 寄存器、不装驱动。

pub mod nvml;
pub mod power;
pub mod profiles;
pub mod storage;

use poise_core::diagnose::Thermals;
use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::SettingStore;
use poise_core::prefs::Mode;

use nvml::Nvml;
use profiles::{FanReading, MachineProfile};
use storage::{MemoryInfo, StorageMonitor, StorageReading};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareReading {
    pub thermals: Thermals,
    pub gpu_pct: Option<f32>,
    pub fans: Vec<FanReading>,
    pub storage: StorageReading,
}

/// 对外的唯一入口。
pub struct Hardware {
    profile: Box<dyn MachineProfile>,
    nvml: Nvml,
    storage: StorageMonitor,
    memory: Option<MemoryInfo>,
    applied: Option<(Mode, bool)>,
}

impl Hardware {
    pub fn detect() -> Self {
        let profile = profiles::detect();
        log::info!("硬件档案：{}", profile.name());
        Self {
            profile,
            nvml: Nvml::default(),
            storage: StorageMonitor::default(),
            memory: storage::read_memory(),
            applied: None,
        }
    }

    pub fn profile_name(&self) -> &str {
        self.profile.name()
    }

    pub fn thermal_control(&self) -> bool {
        self.profile.thermal_control()
    }

    pub fn memory(&self) -> Option<MemoryInfo> {
        self.memory
    }

    /// 读取温度、风扇、硬盘。`detailed` 为假时不访问显卡（避免唤醒休眠中的独显）、不读风扇。
    /// 机型接口能从主板读显卡温度时优先用它，同样不会唤醒独显。
    pub fn read(&mut self, detailed: bool, now: Millis) -> HardwareReading {
        let gpu = if detailed {
            self.nvml.read()
        } else {
            self.nvml.release_if_idle();
            Default::default()
        };
        let machine = self.profile.read(detailed);
        HardwareReading {
            thermals: Thermals {
                cpu_temp: machine.cpu_temp,
                gpu_temp: machine.gpu_temp.or(gpu.temp),
            },
            gpu_pct: gpu.utilization,
            fans: machine.fans,
            storage: self.storage.read(now).clone(),
        }
    }

    /// 模式或供电状态是否与上次应用的不同。
    pub fn needs_apply(&self, mode: Mode, on_battery: bool) -> bool {
        self.applied != Some((mode, on_battery))
    }

    /// 应用模式：电源计划参数（通用）+ 散热模式（机型）。模式没变时不重复写入。
    pub fn apply_mode(
        &mut self,
        mode: Mode,
        on_battery: bool,
        store: &dyn SettingStore,
        journal: &mut Journal,
        now: Millis,
    ) -> Result<(), JournalError> {
        if !self.needs_apply(mode, on_battery) {
            return Ok(());
        }
        power::apply(mode, store, journal, now)?;
        if let Err(e) = self.profile.apply_thermal_mode(mode, on_battery) {
            log::warn!("切换散热模式失败：{e}");
        }
        self.applied = Some((mode, on_battery));
        Ok(())
    }

    /// 还原到原始状态（服务停止时调用）。
    pub fn restore(&mut self, store: &dyn SettingStore, journal: &mut Journal, now: Millis) {
        if let Err(e) = power::apply(Mode::Balanced, store, journal, now) {
            log::warn!("还原电源设置失败：{e}");
        }
        self.profile.restore_thermal_mode();
        self.applied = None;
    }
}
