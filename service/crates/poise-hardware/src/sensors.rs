//! 硬件传感器：温度、风扇、显卡占用、硬盘。只读，设计为在独立的采集线程里使用：
//! 机型接口（WMI）一次读取可能要几十到几百毫秒，不应让调度和界面请求等它。

use poise_core::diagnose::Thermals;
use poise_core::model::Millis;
use poise_platform::observe::gpu::DiscreteActivity;

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
    /// 后台：机型接口最多每 [`BASIC_MACHINE_EVERY_MS`] 读一次处理器温度（详细程度来回变化时也不多读）。
    /// 不访问显卡（避免唤醒休眠中的独显），不读风扇。
    #[default]
    Basic,
    /// 有重点程序（正在用的程序在持续干重活，不是游戏）：完全不调用机型接口（每次调用都可能让所有核心
    /// 短暂停顿），也不访问显卡、不刷新硬盘信息。是否被限速由处理器自己报告，不需要温度。
    Focused,
    /// 玩游戏、界面没显示：完全不调用机型接口（每次调用都可能让所有核心短暂停顿，表现为掉帧）。
    /// 显卡温度和占用从显卡驱动读（独显本来就醒着），不刷新硬盘信息。
    Gaming,
    /// 玩游戏或有重点程序、界面显示着：显卡从显卡驱动读；机型接口最多每 [`WATCHED_MACHINE_EVERY_MS`]
    /// 读一次处理器温度，其间界面显示上次的读数；不读风扇、不刷新硬盘信息。界面开在副屏上玩游戏时
    /// 也不会每隔几秒停顿一下。
    Watched,
    /// 界面显示着、没有要紧的事：全部；风扇转速变化慢，隔一会儿读一次（见 [`FANS_EVERY_MS`]）。
    Full,
}

impl SensorLevel {
    /// 按当前情况选详细程度：要紧的事（游戏、重点程序）优先于界面。
    pub fn choose(ui_visible: bool, gaming: bool, focused: bool) -> Self {
        match (ui_visible, gaming || focused) {
            (true, false) => Self::Full,
            (true, true) => Self::Watched,
            (false, _) if gaming => Self::Gaming,
            (false, _) if focused => Self::Focused,
            _ => Self::Basic,
        }
    }

    /// 界面显示着：读得勤一些（走势图、温度每隔几秒刷新）。
    pub fn shown(self) -> bool {
        matches!(self, Self::Watched | Self::Full)
    }
}

/// 界面显示着时，风扇转速多久读一次。
const FANS_EVERY_MS: Millis = 10_000;

/// 玩游戏或有重点程序、界面显示着时，处理器温度多久从机型接口读一次。
const WATCHED_MACHINE_EVERY_MS: Millis = 30_000;

/// 后台时处理器温度多久从机型接口读一次（采集线程平时 30 秒读一轮，略短一些不错过）。
const BASIC_MACHINE_EVERY_MS: Millis = 20_000;

pub struct Sensors {
    machine: Box<dyn MachineSensors>,
    nvml: Nvml,
    /// 独显有没有醒着（读 NVML 之前看一眼，见 [`DiscreteActivity`]）。
    discrete: DiscreteActivity,
    storage: StorageMonitor,
    /// 最近一次读到的风扇转速与时间。
    fans: Vec<FanReading>,
    fans_at: Option<Millis>,
    /// 最近一次从机型接口读到的处理器温度与时间。
    cpu_temp: Option<f32>,
    cpu_temp_at: Option<Millis>,
}

impl Sensors {
    pub(crate) fn new(machine: Box<dyn MachineSensors>) -> Self {
        Self {
            machine,
            nvml: Nvml::default(),
            discrete: DiscreteActivity::default(),
            storage: StorageMonitor::default(),
            fans: Vec::new(),
            fans_at: None,
            cpu_temp: None,
            cpu_temp_at: None,
        }
    }

    /// 按详细程度读取温度、显卡、风扇、硬盘（见 [`SensorLevel`]）。
    /// 不玩游戏时，机型接口能从主板读显卡温度就优先用它，不会唤醒独显；独显在休眠时也不读 NVML。
    pub fn read(&mut self, level: SensorLevel, now: Millis) -> HardwareReading {
        // 每轮都看一眼独显：只在需要时才看的话，比较的是很久以前的读数，早就睡了的独显也会被当成醒着。
        let asleep = self.discrete.busy() == Some(false);
        let skip_gpu = matches!(level, SensorLevel::Basic | SensorLevel::Focused) || asleep;
        let gpu = if skip_gpu {
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
        if need.cpu_temp {
            self.cpu_temp = machine.cpu_temp;
            self.cpu_temp_at = Some(now);
        }
        let cpu_temp = if matches!(level, SensorLevel::Watched | SensorLevel::Basic) {
            self.cpu_temp
        } else {
            machine.cpu_temp
        };
        let storage = if matches!(level, SensorLevel::Basic | SensorLevel::Full) {
            self.storage.read(now, level == SensorLevel::Full)
        } else {
            self.storage.latest()
        };
        HardwareReading {
            thermals: Thermals {
                cpu_temp,
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
            SensorLevel::Watched => MachineNeed {
                cpu_temp: self
                    .cpu_temp_at
                    .is_none_or(|t| now.saturating_sub(t) >= WATCHED_MACHINE_EVERY_MS),
                ..Default::default()
            },
            SensorLevel::Basic => MachineNeed {
                cpu_temp: self
                    .cpu_temp_at
                    .is_none_or(|t| now.saturating_sub(t) >= BASIC_MACHINE_EVERY_MS),
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
    fn switching_back_to_the_background_does_not_read_again_right_away() {
        let (mut s, calls) = sensors();
        s.read(SensorLevel::Basic, 0);
        s.read(SensorLevel::Gaming, 2_000);
        let r = s.read(SensorLevel::Basic, 4_000);
        assert_eq!(r.thermals.cpu_temp, Some(60.0));
        assert_eq!(calls.lock().unwrap().len(), 1);
        s.read(SensorLevel::Basic, BASIC_MACHINE_EVERY_MS);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn with_the_ui_open_while_gaming_the_machine_interface_is_read_rarely() {
        let (mut s, calls) = sensors();
        assert_eq!(
            s.read(SensorLevel::Watched, 0).thermals.cpu_temp,
            Some(60.0)
        );
        // 其间不调用机型接口，照样显示上次的温度。
        let r = s.read(SensorLevel::Watched, 2_000);
        assert_eq!(r.thermals.cpu_temp, Some(60.0));
        assert!(r.fans.is_empty());
        s.read(SensorLevel::Watched, WATCHED_MACHINE_EVERY_MS);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn games_and_focused_work_come_before_the_ui() {
        assert_eq!(SensorLevel::choose(true, false, false), SensorLevel::Full);
        assert_eq!(SensorLevel::choose(true, true, false), SensorLevel::Watched);
        assert_eq!(SensorLevel::choose(true, false, true), SensorLevel::Watched);
        assert_eq!(SensorLevel::choose(false, true, true), SensorLevel::Gaming);
        assert_eq!(
            SensorLevel::choose(false, false, true),
            SensorLevel::Focused
        );
        assert_eq!(SensorLevel::choose(false, false, false), SensorLevel::Basic);
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
