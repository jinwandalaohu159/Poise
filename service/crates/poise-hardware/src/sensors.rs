//! 硬件传感器：温度、风扇、显卡占用、硬盘。只读，设计为在独立的采集线程里使用：
//! 机型接口（WMI）一次读取可能要几十到几百毫秒，不应让调度和界面请求等它。

use poise_core::diagnose::Thermals;
use poise_core::model::Millis;

use crate::nvml::Nvml;
use crate::profiles::{FanReading, MachineNeed, MachineSensors};
use crate::storage::{StorageMonitor, StorageReading};

/// 一次传感器读数。读不到的项为空。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareReading {
    pub thermals: Thermals,
    pub gpu_pct: Option<f32>,
    pub fans: Vec<FanReading>,
    pub storage: StorageReading,
}

/// 读取的详细程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SensorLevel {
    /// 后台：机型接口读温度。不访问显卡（避免唤醒休眠中的独显），不读风扇。
    #[default]
    Basic,
    /// 玩游戏、界面没显示：机型接口只读处理器温度（每次调用都可能让所有核心短暂停顿，表现为掉帧），
    /// 显卡温度和占用从显卡驱动读（独显本来就醒着），不读风扇，不刷新硬盘信息。
    Gaming,
    /// 界面显示着：全部。
    Full,
}

pub struct Sensors {
    machine: Box<dyn MachineSensors>,
    nvml: Nvml,
    storage: StorageMonitor,
}

impl Sensors {
    pub(crate) fn new(machine: Box<dyn MachineSensors>) -> Self {
        Self {
            machine,
            nvml: Nvml::default(),
            storage: StorageMonitor::default(),
        }
    }

    /// 按详细程度读取温度、显卡、风扇、硬盘（见 [`SensorLevel`]）。
    /// 不玩游戏时，机型接口能从主板读显卡温度就优先用它，不会唤醒独显。
    pub fn read(&mut self, level: SensorLevel, now: Millis) -> HardwareReading {
        let gpu = if level == SensorLevel::Basic {
            self.nvml.release_if_idle();
            Default::default()
        } else {
            self.nvml.read()
        };
        let machine = self.machine.read(MachineNeed {
            gpu_temp: level != SensorLevel::Gaming || gpu.temp.is_none(),
            fans: level == SensorLevel::Full,
        });
        let storage = if level == SensorLevel::Gaming {
            self.storage.latest()
        } else {
            self.storage.read(now)
        };
        HardwareReading {
            thermals: Thermals {
                cpu_temp: machine.cpu_temp,
                gpu_temp: machine.gpu_temp.or(gpu.temp),
            },
            gpu_pct: gpu.utilization,
            fans: machine.fans,
            storage: storage.clone(),
        }
    }
}
