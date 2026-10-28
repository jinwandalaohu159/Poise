//! 硬件适配。
//!
//! - 通用层：电源计划参数（[`power`]）、NVIDIA 显卡读数（[`nvml`]）、硬盘（[`storage`]）、内存规格（[`memory`]），任何电脑都能用；
//! - 机型层：[`profiles`]，每台电脑一份档案，提供散热模式、温度、风扇等机型特有能力。
//!
//! 控制（[`Hardware`]：模式、风扇加速）与读数（[`Sensors`]：温度、风扇、显卡、硬盘）分开：
//! 读数在独立的采集线程里进行，控制在调度引擎里进行，两边使用各自的接口连接。
//!
//! 边界：只用官方接口与已验证的取值；不超频、不降压、不写 CPU 寄存器、不装驱动。

pub mod memory;
pub mod nvml;
pub mod power;
pub mod profiles;
pub mod sensors;
pub mod storage;

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::SettingStore;
use poise_core::prefs::Mode;

use memory::MemoryInfo;
use profiles::MachineProfile;
pub use sensors::{HardwareReading, Sensors};

/// 传感器读数是否是合理的摄氏温度（排除 0 和明显的错误值）。
fn plausible_celsius(t: u64) -> bool {
    (1..=125).contains(&t)
}

/// 硬件控制（模式、风扇加速）与静态信息。读数由 [`Sensors`] 在采集线程里读取。
pub struct Hardware {
    profile: Box<dyn MachineProfile>,
    memory: Option<MemoryInfo>,
    applied: Option<(Mode, bool)>,
    /// 已经写入的风扇加速（百分比）；None 表示还没写过。
    applied_boost: Option<u8>,
}

impl Hardware {
    pub fn detect() -> Self {
        let profile = profiles::detect();
        log::info!("硬件档案：{}", profile.name());
        Self {
            profile,
            memory: memory::read_memory(),
            applied: None,
            applied_boost: None,
        }
    }

    pub fn profile_name(&self) -> &str {
        self.profile.name()
    }

    pub fn thermal_control(&self) -> bool {
        self.profile.thermal_control()
    }

    /// 机型极限模式的标识（例如 "dell_g_mode"）；不支持时为 None。
    pub fn vendor_mode(&self) -> Option<&'static str> {
        self.profile.vendor_mode()
    }

    pub fn fan_boost_supported(&self) -> bool {
        self.profile.fan_boost_supported()
    }

    /// 风扇加速（0～100，0 = 不加速）。与上次写入的相同时不重复写。
    pub fn apply_fan_boost(&mut self, percent: u8) {
        if !self.fan_boost_supported() || self.applied_boost == Some(percent) {
            return;
        }
        match self.profile.set_fan_boost(percent) {
            Ok(()) => self.applied_boost = Some(percent),
            Err(e) => log::warn!("风扇加速失败：{e}"),
        }
    }

    pub fn memory(&self) -> Option<&MemoryInfo> {
        self.memory.as_ref()
    }

    /// 一份只读的传感器（独立连接），交给采集线程使用。
    pub fn sensors(&self) -> Sensors {
        Sensors::new(self.profile.sensors())
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
        // BIOS 切换散热模式时会把风扇加速清零（G15 实测），之后需要重新写入。
        self.applied_boost = None;
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
        if self.applied_boost.is_some_and(|p| p > 0) {
            self.apply_fan_boost(0);
        }
        self.applied_boost = None;
    }
}
