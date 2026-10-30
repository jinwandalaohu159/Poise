//! 硬件传感器：温度、风扇、显卡占用、硬盘。只读，设计为在独立的采集线程里使用：
//! 机型接口（WMI）一次读取可能要几十到几百毫秒，不应让调度和界面请求等它。

use poise_core::diagnose::Thermals;
use poise_core::model::Millis;

use crate::nvml::Nvml;
use crate::profiles::{FanReading, MachineNeed, MachineReading, MachineSensors};
use crate::storage::{StorageMonitor, StorageReading};

/// 一次传感器读数。读不到的项为空。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareReading {
    pub thermals: Thermals,
    pub gpu_pct: Option<f32>,
    pub fans: Vec<FanReading>,
    pub storage: StorageReading,
}

/// 读取的详细程度。处理器是否被限速由处理器自己报告（见 `poise_platform::observe::cpu_performance`），
/// 不靠温度推测，所以界面没显示时温度只用于健康状态和走势，少读即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SensorLevel {
    /// 后台：机型接口只读处理器温度。不访问显卡（避免唤醒休眠中的独显），不读风扇。
    #[default]
    Basic,
    /// 有重点程序（正在用的程序在持续干重活，不是游戏）：完全不调用机型接口（每次调用都可能让所有核心
    /// 短暂停顿），也不访问显卡、不刷新硬盘信息。是否被限速由处理器自己报告，不需要温度。
    Focused,
    /// 玩游戏、界面没显示：完全不调用机型接口（每次调用都可能让所有核心短暂停顿，表现为掉帧）。
    /// 显卡温度和占用从显卡驱动读（独显本来就醒着），不刷新硬盘信息。
    Gaming,
    /// 界面显示着：全部；风扇转速变化慢，隔一会儿读一次（见 [`FANS_EVERY_MS`]）。
    Full,
}

/// 界面显示着时，风扇转速多久读一次。
const FANS_EVERY_MS: Millis = 10_000;

pub struct Sensors {
    machine: Box<dyn MachineSensors>,
    nvml: Nvml,
    storage: StorageMonitor,
    /// 最近一次读到的风扇转速与时间。
    fans: Vec<FanReading>,
    fans_at: Option<Millis>,
}

impl Sensors {
    pub(crate) fn new(machine: Box<dyn MachineSensors>) -> Self {
        Self {
            machine,
            nvml: Nvml::default(),
            storage: StorageMonitor::default(),
            fans: Vec::new(),
            fans_at: None,
        }
    }

    /// 按详细程度读取温度、显卡、风扇、硬盘（见 [`SensorLevel`]）。
    /// 不玩游戏时，机型接口能从主板读显卡温度就优先用它，不会唤醒独显。
    pub fn read(&mut self, level: SensorLevel, now: Millis) -> HardwareReading {
        let gpu = if matches!(level, SensorLevel::Basic | SensorLevel::Focused) {
            self.nvml.release_if_idle();
            Default::default()
        } else {
            self.nvml.read()
        };
        let need = self.machine_need(level, now);
        let machine = if need.nothing() {
            MachineReading::default()
        } else {
            self.machine.read(need)
        };
        if need.fans {
            self.fans = machine.fans.clone();
            self.fans_at = Some(now);
        } else if level != SensorLevel::Full {
            self.fans.clear();
            self.fans_at = None;
        }
        let storage = if matches!(level, SensorLevel::Gaming | SensorLevel::Focused) {
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
            fans: self.fans.clone(),
            storage: storage.clone(),
        }
    }

    /// 这一次要从机型接口读的项（见 [`SensorLevel`]）。
    fn machine_need(&self, level: SensorLevel, now: Millis) -> MachineNeed {
        match level {
            SensorLevel::Gaming | SensorLevel::Focused => MachineNeed::default(),
            SensorLevel::Basic => MachineNeed {
                cpu_temp: true,
                ..Default::default()
            },
            SensorLevel::Full => MachineNeed {
                cpu_temp: true,
                gpu_temp: true,
                fans: self
                    .fans_at
                    .is_none_or(|t| now.saturating_sub(t) >= FANS_EVERY_MS),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// 记下每次被要求读哪些项。
    struct Recorder(Arc<Mutex<Vec<MachineNeed>>>);

    impl MachineSensors for Recorder {
        fn read(&mut self, need: MachineNeed) -> MachineReading {
            self.0.lock().unwrap().push(need);
            MachineReading {
                cpu_temp: need.cpu_temp.then_some(60.0),
                fans: if need.fans {
                    vec![FanReading {
                        rpm: 2000,
                        max_rpm: 5000,
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            }
        }
    }

    fn sensors() -> (Sensors, Arc<Mutex<Vec<MachineNeed>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (Sensors::new(Box::new(Recorder(calls.clone()))), calls)
    }

    #[test]
    fn gaming_and_focused_work_never_call_the_machine_interface() {
        let (mut s, calls) = sensors();
        s.read(SensorLevel::Gaming, 0);
        s.read(SensorLevel::Focused, 2_000);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn background_reads_only_the_cpu_temperature() {
        let (mut s, calls) = sensors();
        let r = s.read(SensorLevel::Basic, 0);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [MachineNeed {
                cpu_temp: true,
                ..Default::default()
            }]
        );
        assert_eq!(r.thermals.cpu_temp, Some(60.0));
    }

    #[test]
    fn with_the_ui_open_fans_are_read_every_few_seconds_and_shown_in_between() {
        let (mut s, calls) = sensors();
        assert_eq!(s.read(SensorLevel::Full, 0).fans.len(), 1);
        // 两秒后：不读风扇，但照样显示上次的转速。
        assert_eq!(s.read(SensorLevel::Full, 2_000).fans.len(), 1);
        s.read(SensorLevel::Full, FANS_EVERY_MS);
        let fans: Vec<bool> = calls.lock().unwrap().iter().map(|n| n.fans).collect();
        assert_eq!(fans, [true, false, true]);
    }
}
