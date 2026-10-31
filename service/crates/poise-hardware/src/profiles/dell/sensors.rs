//! 戴尔的只读传感器：独立的 AWCC 连接，在采集线程里读温度和风扇。

use std::collections::HashMap;

use super::super::awcc::Awcc;
use super::super::{FanReading, MachineNeed, MachineReading, MachineSensors};

/// 戴尔的只读传感器：独立的 AWCC 连接，只读温度和风扇。
pub(super) struct DellSensors {
    awcc: Option<Awcc>,
    /// 各风扇的最高转速：不会变，第一次读到后记住，之后不再调用。
    max_rpm: HashMap<u8, u32>,
}

impl MachineSensors for DellSensors {
    fn read(&mut self, need: MachineNeed) -> MachineReading {
        let Some(awcc) = &self.awcc else {
            return MachineReading::default();
        };
        // BIOS 报告的传感器顺序：CPU、显卡。
        let sensors = &awcc.layout().sensors;
        let fans = if need.fans {
            let max_rpm = &mut self.max_rpm;
            awcc.layout()
                .fans
                .iter()
                .filter_map(|&fan| {
                    let max = match max_rpm.get(&fan) {
                        Some(&m) => m,
                        None => {
                            let m = awcc.fan_max_rpm(fan)?;
                            max_rpm.insert(fan, m);
                            m
                        }
                    };
                    Some(FanReading {
                        rpm: awcc.fan_rpm(fan)?,
                        max_rpm: max,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        MachineReading {
            cpu_temp: need
                .cpu_temp
                .then(|| sensors.first().and_then(|&s| awcc.temperature(s)))
                .flatten(),
            gpu_temp: need
                .gpu_temp
                .then(|| sensors.get(1).and_then(|&s| awcc.temperature(s)))
                .flatten(),
            fans,
        }
    }
}

impl DellSensors {
    pub(super) fn new(awcc: Option<Awcc>) -> Self {
        Self {
            awcc,
            max_rpm: HashMap::new(),
        }
    }
}
