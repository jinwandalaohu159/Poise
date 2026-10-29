//! 硬件适配。
//!
//! - 通用层：电源计划参数（[`power`]）、NVIDIA 显卡读数（[`nvml`]）、硬盘（[`storage`]）、内存规格（[`memory`]），任何电脑都能用；
//! - 机型层：[`profiles`]，每台电脑一份档案，提供散热模式、温度、风扇等机型特有能力。
//!
//! 控制（[`Hardware`]：模式、风扇加速）与读数（[`Sensors`]：温度、风扇、显卡、硬盘）分开：
//! 读数在独立的采集线程里进行；控制由调度引擎决定，慢的硬件操作交给执行线程（`actuator`），
//! 调度引擎从不等待硬件。两边使用各自的接口连接。
//!
//! 边界：只用官方接口与已验证的取值；不超频、不降压、不写 CPU 寄存器、不装驱动。

mod actuator;
pub mod memory;
pub mod nvml;
pub mod power;
pub mod profiles;
pub mod sensors;
pub mod storage;

use std::time::Duration;

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::SettingStore;
use poise_core::prefs::Mode;

use actuator::Actuator;
use memory::MemoryInfo;
pub use sensors::{HardwareReading, SensorLevel, Sensors};

/// 传感器读数是否是合理的摄氏温度（排除 0 和明显的错误值）。
fn plausible_celsius(t: u64) -> bool {
    (1..=125).contains(&t)
}

/// 还原硬件时最多等执行线程多久（BIOS 切换散热模式约 1 秒）。
const RESTORE_TIMEOUT: Duration = Duration::from_secs(5);

/// 机型档案的静态能力（档案本身交给执行线程）。
struct Capabilities {
    name: String,
    thermal_control: bool,
    vendor_mode: Option<&'static str>,
    fan_boost_supported: bool,
}

/// 硬件控制（模式、风扇加速）与静态信息。读数由 [`Sensors`] 在采集线程里读取。
pub struct Hardware {
    caps: Capabilities,
    actuator: Actuator,
    /// 交给采集线程的传感器（只取一次）。
    sensors: Option<Sensors>,
    memory: Option<MemoryInfo>,
    applied: Option<(Mode, bool)>,
    /// 已经写入的风扇加速（百分比）；None 表示还没写过。
    applied_boost: Option<u8>,
    /// 大核的效率等级（不是大小核时为 0），电源计划按它区分大小核。
    top_class: u8,
}

impl Hardware {
    /// `top_class`：大核的效率等级（见 `CpuTopology::top_class`）。
    pub fn detect(top_class: u8) -> Self {
        let profile = profiles::detect();
        log::info!("硬件档案：{}", profile.name());
        let caps = Capabilities {
            name: profile.name().to_string(),
            thermal_control: profile.thermal_control(),
            vendor_mode: profile.vendor_mode(),
            fan_boost_supported: profile.fan_boost_supported(),
        };
        let sensors = Sensors::new(profile.sensors());
        Self {
            caps,
            actuator: Actuator::start(profile),
            sensors: Some(sensors),
            memory: memory::read_memory(),
            applied: None,
            applied_boost: None,
            top_class,
        }
    }

    pub fn profile_name(&self) -> &str {
        &self.caps.name
    }

    pub fn thermal_control(&self) -> bool {
        self.caps.thermal_control
    }

    /// 机型极限模式的标识（例如 "dell_g_mode"）；不支持时为 None。
    pub fn vendor_mode(&self) -> Option<&'static str> {
        self.caps.vendor_mode
    }

    pub fn fan_boost_supported(&self) -> bool {
        self.caps.fan_boost_supported
    }

    /// 风扇加速（0～100，0 = 不加速）。与上次写入的相同时不重复写。在执行线程里写入。
    pub fn apply_fan_boost(&mut self, percent: u8) {
        if !self.fan_boost_supported() || self.applied_boost == Some(percent) {
            return;
        }
        self.actuator.fan_boost(percent);
        self.applied_boost = Some(percent);
    }

    pub fn memory(&self) -> Option<&MemoryInfo> {
        self.memory.as_ref()
    }

    /// 只读的传感器（独立连接），交给采集线程使用。只能取一次。
    pub fn take_sensors(&mut self) -> Sensors {
        self.sensors.take().expect("传感器只能取一次")
    }

    /// 模式或供电状态是否与上次应用的不同。
    pub fn needs_apply(&self, mode: Mode, on_battery: bool) -> bool {
        self.applied != Some((mode, on_battery))
    }

    /// 应用模式：电源计划参数（通用）+ 散热模式（机型）。模式没变时不重复写入。
    /// 电源设置经变更日志写入（保证可还原）；让它生效、切换散热模式都在执行线程里进行，这里不等待。
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
        let (written, activate) = power::write(mode, self.top_class, store, journal, now);
        if activate {
            self.actuator.activate_power();
        }
        written?;
        self.actuator.thermal(mode, on_battery);
        // BIOS 切换散热模式时会把风扇加速清零（G15 实测），之后需要重新写入。
        self.applied_boost = None;
        self.applied = Some((mode, on_battery));
        Ok(())
    }

    /// 还原到原始状态（服务停止、Poise 退出时调用），等硬件真正还原完成。
    pub fn restore(&mut self, store: &dyn SettingStore, journal: &mut Journal, now: Millis) {
        let (written, activate) = power::write(Mode::Balanced, self.top_class, store, journal, now);
        if let Err(e) = written {
            log::warn!("还原电源设置失败：{e}");
        }
        if activate {
            self.actuator.activate_power();
        }
        let boost_on = self.applied_boost.is_some_and(|p| p > 0);
        self.actuator.restore(boost_on, RESTORE_TIMEOUT);
        self.applied = None;
        self.applied_boost = None;
    }
}
