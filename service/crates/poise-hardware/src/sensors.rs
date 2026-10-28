//! 硬件传感器：温度、风扇、显卡占用、硬盘。只读，设计为在独立的采集线程里使用：
//! 机型接口（WMI）一次读取可能要几十到几百毫秒，不应让调度和界面请求等它。

use poise_core::diagnose::Thermals;
use poise_core::model::Millis;

use crate::nvml::Nvml;
use crate::profiles::{FanReading, MachineSensors};
use crate::storage::{StorageMonitor, StorageReading};

/// 一次传感器读数。读不到的项为空。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareReading {
    pub thermals: Thermals,
    pub gpu_pct: Option<f32>,
    pub fans: Vec<FanReading>,
    pub storage: StorageReading,
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

    /// 读取温度、风扇、硬盘。`detailed` 为假时不访问显卡（避免唤醒休眠中的独显）、不读风扇。
    /// 机型接口能从主板读显卡温度时优先用它，同样不会唤醒独显。
    pub fn read(&mut self, detailed: bool, now: Millis) -> HardwareReading {
        let gpu = if detailed {
            self.nvml.read()
        } else {
            self.nvml.release_if_idle();
            Default::default()
        };
        let machine = self.machine.read(detailed);
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
}
