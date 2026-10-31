//! 硬件适配。
//!
//! - 通用层：电源计划参数（[`power`]）、NVIDIA 显卡读数（[`nvml`]）、硬盘（[`storage`]）、内存规格（[`memory`]），任何电脑都能用；
//! - 机型层：[`profiles`]，每台电脑一份档案，提供散热模式、温度、风扇等机型特有能力。
//!
//! 控制（[`Hardware`]：模式、风扇加速）与读数（[`Sensors`]：温度、风扇、显卡、硬盘）分开：
//! 读数在独立的采集线程里进行；控制由调度引擎决定，慢的硬件操作交给执行线程（`actuator`：电源设置一条线、
//! BIOS 一条线），调度引擎从不等待硬件。两边使用各自的接口连接。
//!
//! 边界：只用官方接口与已验证的取值；不超频、不降压、不写 CPU 寄存器、不装驱动。

mod actuator;
pub mod memory;
pub mod nvml;
pub mod power;
pub mod profiles;
pub mod sensors;
pub mod storage;

use std::path::Path;
use std::time::{Duration, Instant};

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::prefs::Mode;
use poise_core::setting::SettingStore;

use actuator::Actuator;
pub use actuator::PowerWriter;
use memory::MemoryInfo;
pub use power::{PendingWrites, Staged};
pub use sensors::{HardwareReading, SensorLevel, Sensors};

/// 传感器读数是否是合理的摄氏温度（排除 0 和明显的错误值）。
fn plausible_celsius(t: u64) -> bool {
    (1..=125).contains(&t)
}

/// 服务停止时最多等 BIOS 还原多久（切换散热模式约 1 秒）。
const BIOS_TIMEOUT: Duration = Duration::from_secs(5);
/// 服务停止时最多等排着的电源设置写完多久（电源服务忙时每项几百毫秒，撤销一个模式二三十项）。
const POWER_TIMEOUT: Duration = Duration::from_secs(10);
/// 预读交给执行线程这么久还没读完时记一条日志（电源服务卡住了）。
const PREFETCH_OVERDUE: Duration = Duration::from_secs(15);
/// 切换散热模式失败后过这么久再切一次。
const THERMAL_RETRY: Duration = Duration::from_secs(30);

/// 应用模式的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// 和上次应用的相同，什么都没做。
    Unchanged,
    /// 已经记好、交给执行线程写入。
    Written,
    /// 第一次接管，原值还在执行线程里预读（见 `power::Prefetched`），读好后再写（调用方应尽快再来）。
    Waiting,
}

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
    /// 上次要求的风扇加速（百分比，0 = 不管；BIOS 执行线程记着真正写入的，见 `actuator::bios`）。
    requested_boost: u8,
    /// 切换散热模式失败后，什么时候再切一次。
    thermal_retry_at: Option<Instant>,
    /// 处理器的组成，电源计划按它区分大小核。
    cpu: power::CpuShape,
    /// Windows 平衡计划在这台电脑上的出厂值（均衡模式与所有模式的底线按它设置）。
    baseline: power::Baseline,
}

impl Hardware {
    /// `data_dir`：机型档案在这里记下需要跨重启保存的状态（例如接管前的散热模式）。
    pub fn detect(cpu: power::CpuShape, data_dir: &Path) -> Self {
        let profile = profiles::detect(data_dir);
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
            requested_boost: 0,
            thermal_retry_at: None,
            cpu,
            baseline: power::Baseline::read(),
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

    /// 风扇加速（0～100，0 = 不管）。与上次要求的相同时不再发出。在执行线程里写入。
    pub fn apply_fan_boost(&mut self, percent: u8) {
        if !self.fan_boost_supported() || self.requested_boost == percent {
            return;
        }
        self.actuator.fan_boost(percent);
        self.requested_boost = percent;
    }

    /// 电脑睡眠醒来：核对散热模式和风扇加速还是不是 Poise 写的（BIOS 可能改了），不是就重新写入。
    pub fn resumed(&self) {
        if self.applied.is_some() || self.requested_boost > 0 {
            self.actuator.recheck_bios();
        }
    }

    /// 切换散热模式失败了（WMI 一时不可用）：过 [`THERMAL_RETRY`] 再切一次，不等到下次换模式。
    fn retry_failed_thermal(&mut self) {
        if self.actuator.take_thermal_failure() {
            self.thermal_retry_at
                .get_or_insert_with(|| Instant::now() + THERMAL_RETRY);
        }
        if let (Some(at), Some((mode, on_battery))) = (self.thermal_retry_at, self.applied)
            && Instant::now() >= at
        {
            self.thermal_retry_at = None;
            self.actuator.thermal(mode, on_battery);
        }
    }

    pub fn memory(&self) -> Option<&MemoryInfo> {
        self.memory.as_ref()
    }

    /// 只读的传感器（独立连接），交给采集线程使用。只能取一次。
    pub fn take_sensors(&mut self) -> Sensors {
        self.sensors.take().expect("传感器只能取一次")
    }

    /// 电源设置的写入口（见 [`PowerWriter`]）：所有电源设置的修改都经它交给执行线程。
    pub fn power_writer(&self) -> PowerWriter {
        self.actuator.power().clone()
    }

    /// 电源计划换了（用户或别的程序切换了计划）：下次应用模式时重新写入新计划。
    /// 上一个计划里写过的值由变更日志写回那个计划。
    pub fn power_plan_changed(&mut self) {
        self.applied = None;
        self.actuator.power().prefetched().clear();
    }

    /// 一个模式写入的电源设置（日志用，见 [`power::describe`]）。
    pub fn describe_power(&self, mode: Mode, on_battery: bool) -> String {
        power::describe(mode, self.cpu, &self.baseline, !on_battery)
    }

    /// 模式或供电状态是否与上次应用的不同。
    pub fn needs_apply(&self, mode: Mode, on_battery: bool) -> bool {
        self.applied != Some((mode, on_battery))
    }

    /// 应用模式：电源计划参数（通用）+ 散热模式（机型）。模式没变时不重复写入。
    /// 电源设置经变更日志写入（保证可还原），`store` 要经过 [`PowerWriter::stage`]：真正写进电源服务、
    /// 让它生效、切换散热模式都在执行线程里进行，这里不等待。
    ///
    /// 第一次接管（服务启动、Poise 重新打开、换了电源计划）要先读出每项的原值：交给执行线程预读，
    /// 这次先不写（[`Applied::Waiting`]），读好之后再应用时直接用。
    pub fn apply_mode(
        &mut self,
        mode: Mode,
        on_battery: bool,
        store: &dyn SettingStore,
        journal: &mut Journal,
        now: Millis,
    ) -> Result<Applied, JournalError> {
        self.retry_failed_thermal();
        if !self.needs_apply(mode, on_battery) {
            return Ok(Applied::Unchanged);
        }
        let power = self.actuator.power();
        if self.applied.is_none() && !power.prefetched().ready() {
            if let Some(generation) = power.prefetched().ask() {
                let wanted = power::settings_for(mode, self.cpu, &self.baseline);
                power.prefetch(generation, wanted.into_iter().map(|(t, _)| t).collect());
            } else if power.prefetched().newly_overdue(PREFETCH_OVERDUE) {
                log::warn!(
                    "预读电源设置超过 {} 秒还没完成（电源服务没有响应），模式等它读完再切换",
                    PREFETCH_OVERDUE.as_secs()
                );
            }
            return Ok(Applied::Waiting);
        }
        let (written, activate) = power::write(mode, self.cpu, &self.baseline, store, journal, now);
        power.prefetched().forget_values();
        // 经过 PowerWriter 的修改随发布交给执行线程激活；直接写进当前计划的才在这里要求激活。
        if activate {
            power.activate();
        }
        written?;
        self.actuator.thermal(mode, on_battery);
        self.applied = Some((mode, on_battery));
        Ok(Applied::Written)
    }

    /// 还原到原始状态（服务停止、Poise 退出时调用），不等待。`store` 同 [`Hardware::apply_mode`]：撤销排在
    /// 之前的写入后面，执行线程按先后写完，不会撤销之后又被写回去。要等还原完成时之后调用 [`Hardware::settle`]。
    pub fn restore(&mut self, store: &dyn SettingStore, journal: &mut Journal) {
        let (written, activate) = power::revert(store, journal);
        if let Err(e) = written {
            log::warn!("还原电源设置失败：{e}");
        }
        if activate {
            self.actuator.power().activate();
        }
        self.actuator.restore_bios();
        self.actuator.power().prefetched().clear();
        self.applied = None;
        // 已经还原了：之前失败的切换不再重试。
        self.thermal_retry_at = None;
        let _ = self.actuator.take_thermal_failure();
        self.requested_boost = 0;
    }

    /// 等执行线程做完之前交给它的事（最多十秒）：服务停止前，排着的电源设置要写完、散热模式要还原。
    pub fn settle(&self) {
        self.actuator.settle(POWER_TIMEOUT, BIOS_TIMEOUT);
    }
}
