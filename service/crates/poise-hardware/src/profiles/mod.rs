//! 机型档案。每台电脑一份，新增机型只需在这里加一个文件并登记到 [`detect`]。

mod awcc;
mod bios;
mod dell_g15;

use bios::read_bios;
use poise_core::prefs::Mode;

use dell_g15::DellG15;

/// 一个风扇的转速与最高转速。各机型档案都返回这个结构。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanReading {
    pub rpm: u32,
    pub max_rpm: u32,
}

/// 机型接口读到的数据。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MachineReading {
    pub cpu_temp: Option<f32>,
    /// 从主板读的显卡温度：不需要唤醒独显。
    pub gpu_temp: Option<f32>,
    pub fans: Vec<FanReading>,
}

/// 机型特有的能力（通用层之外的部分）。
pub trait MachineProfile: Send {
    fn name(&self) -> &str;

    /// 是否能切换散热模式。
    fn thermal_control(&self) -> bool {
        false
    }

    /// 切换散热模式。默认不做任何事。
    fn apply_thermal_mode(&mut self, _mode: Mode, _on_battery: bool) -> Result<(), String> {
        Ok(())
    }

    /// 回到 Poise 启动前的散热模式（服务停止时调用）。
    fn restore_thermal_mode(&mut self) {}

    /// 机型极限模式（[`Mode::Vendor`]）的标识，例如 "dell_g_mode"；界面按它显示名称。
    /// 不支持时为 None，界面不显示这一档。
    fn vendor_mode(&self) -> Option<&'static str> {
        None
    }

    /// 是否能给风扇加速。
    fn fan_boost_supported(&self) -> bool {
        false
    }

    /// 风扇加速，`percent` 为 0～100（0 = 不加速，由 BIOS 按温度调节）。默认不做任何事。
    fn set_fan_boost(&mut self, _percent: u8) -> Result<(), String> {
        Ok(())
    }

    /// 一份独立连接的只读传感器，在采集线程里使用，读取时不占用控制接口。
    fn sensors(&self) -> Box<dyn MachineSensors> {
        Box::new(NoSensors)
    }
}

/// 机型传感器：只读。
pub trait MachineSensors: Send {
    /// 读取温度和风扇。`include_fans` 为假时跳过风扇，减少调用。
    fn read(&mut self, include_fans: bool) -> MachineReading;
}

/// 机型没有传感器接口时使用。
struct NoSensors;

impl MachineSensors for NoSensors {
    fn read(&mut self, _include_fans: bool) -> MachineReading {
        MachineReading::default()
    }
}

/// 没有匹配档案时使用：只有通用层能力。
pub struct Generic;

impl MachineProfile for Generic {
    fn name(&self) -> &str {
        "Generic"
    }
}

pub fn detect() -> Box<dyn MachineProfile> {
    let bios = read_bios();
    if DellG15::matches(&bios) {
        return Box::new(DellG15::new(&bios));
    }
    Box::new(Generic)
}
