//! 机型档案。每台电脑一份，新增机型只需在这里加一个文件并登记到 [`detect`]。

mod awcc;
mod bios;
mod dell_g15;

use bios::read_bios;
use poise_core::prefs::Mode;

pub use awcc::FanReading;
pub use dell_g15::DellG15;

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

    /// 读取温度和风扇（需要机型提供的接口）。`include_fans` 为假时跳过风扇，减少调用。
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
