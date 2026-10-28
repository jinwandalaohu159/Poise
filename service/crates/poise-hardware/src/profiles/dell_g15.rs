//! Dell G15 系列档案：通过 BIOS 的 AWCC 接口切换散热模式、风扇加速、G 模式；温度和风扇由独立连接的只读传感器读取。
//!
//! 散热模式只用 BIOS 自己列出的取值，并且三种目标模式都在列表里才启用切换；
//! "均衡"不强加任何模式，而是回到 Poise 启动时 BIOS 所处的模式（用户在 AWCC 里的选择）。
//! G 模式（机型极限模式）需要 BIOS 提供 G 模式开关；离开 G 模式时先关开关，再切到目标模式。

use std::collections::HashMap;

use poise_core::prefs::Mode;

use super::awcc::{Awcc, G_MODE};
use super::bios::BiosInfo;
use super::{FanReading, MachineNeed, MachineProfile, MachineReading, MachineSensors};

/// G15 5520 BIOS 列出的散热模式（实测：A0、A1、A3、A5）。
const BALANCED: u8 = 0xA0;
const PERFORMANCE: u8 = 0xA1;
const QUIET: u8 = 0xA3;

/// 散热目标：BIOS 散热模式，或 G 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Thermal {
    Mode(u8),
    GMode,
}

pub struct DellG15 {
    name: String,
    awcc: Option<Awcc>,
    /// Poise 启动时 BIOS 的散热模式，"均衡"和服务停止时回到它。
    original_mode: Option<u8>,
    /// BIOS 有 G 模式开关。
    g_mode: bool,
}

impl DellG15 {
    pub fn matches(bios: &BiosInfo) -> bool {
        bios.manufacturer.to_lowercase().starts_with("dell") && bios.product.contains("G15")
    }

    pub fn new(bios: &BiosInfo) -> Self {
        let awcc = match Awcc::open() {
            Ok(a) => {
                log::info!("AWCC 接口：{:?}", a.layout());
                Some(a)
            }
            Err(e) => {
                log::info!("AWCC 接口不可用（{e}），只使用通用能力");
                None
            }
        };
        let g_state = awcc.as_ref().and_then(Awcc::g_mode);
        // 上次在 G 模式下异常退出：先关掉，不把 G 模式当成用户原来的模式。
        if g_state == Some(true)
            && let Some(a) = &awcc
            && let Err(e) = a.set_g_mode(false)
        {
            log::warn!("关闭遗留的 G 模式失败：{e}");
        }
        let original_mode = awcc
            .as_ref()
            .and_then(Awcc::power_mode)
            .map(|m| if m == G_MODE { BALANCED } else { m });
        Self {
            name: bios.product.clone(),
            awcc,
            original_mode,
            g_mode: g_state.is_some(),
        }
    }

    fn target(&self, mode: Mode, on_battery: bool) -> Thermal {
        match mode {
            Mode::Quiet => Thermal::Mode(QUIET),
            // 用电池时不加大散热功耗，只靠电源计划参数。
            Mode::Vendor if !on_battery && self.g_mode => Thermal::GMode,
            Mode::Full | Mode::Vendor if !on_battery => Thermal::Mode(PERFORMANCE),
            _ => Thermal::Mode(self.original_mode.unwrap_or(BALANCED)),
        }
    }

    fn set(&self, target: Thermal) -> Result<(), String> {
        let awcc = self.awcc.as_ref().ok_or("AWCC 接口不可用")?;
        match target {
            Thermal::GMode => {
                if awcc.g_mode() != Some(true) || awcc.power_mode() != Some(G_MODE) {
                    awcc.set_g_mode(true)?;
                    log::info!("散热模式：G 模式");
                }
            }
            Thermal::Mode(mode) => {
                if self.g_mode && awcc.g_mode() == Some(true) {
                    awcc.set_g_mode(false)?;
                }
                if awcc.power_mode() != Some(mode) {
                    awcc.set_power_mode(mode)?;
                    log::info!("散热模式：{mode:#x}");
                }
            }
        }
        Ok(())
    }
}

impl MachineProfile for DellG15 {
    fn name(&self) -> &str {
        &self.name
    }

    fn thermal_control(&self) -> bool {
        self.awcc.as_ref().is_some_and(|a| {
            let modes = &a.layout().power_modes;
            [BALANCED, PERFORMANCE, QUIET]
                .iter()
                .all(|m| modes.contains(m))
        })
    }

    fn vendor_mode(&self) -> Option<&'static str> {
        (self.thermal_control() && self.g_mode).then_some("dell_g_mode")
    }

    fn apply_thermal_mode(&mut self, mode: Mode, on_battery: bool) -> Result<(), String> {
        if !self.thermal_control() {
            return Ok(());
        }
        self.set(self.target(mode, on_battery))
    }

    fn restore_thermal_mode(&mut self) {
        if self.thermal_control()
            && let Some(original) = self.original_mode
            && let Err(e) = self.set(Thermal::Mode(original))
        {
            log::warn!("还原散热模式失败：{e}");
        }
    }

    fn fan_boost_supported(&self) -> bool {
        self.awcc
            .as_ref()
            .is_some_and(|a| !a.layout().fans.is_empty())
    }

    fn set_fan_boost(&mut self, percent: u8) -> Result<(), String> {
        let awcc = self.awcc.as_ref().ok_or("AWCC 接口不可用")?;
        let value = boost_value(percent);
        let mut written = false;
        for &fan in &awcc.layout().fans {
            if awcc.fan_boost(fan) != Some(value) {
                awcc.set_fan_boost(fan, value)?;
                written = true;
            }
        }
        if written {
            log::info!("风扇加速：{percent}%");
        }
        Ok(())
    }

    fn sensors(&self) -> Box<dyn MachineSensors> {
        let awcc = self.awcc.as_ref().and_then(|a| match a.reconnect() {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!("AWCC 读取连接失败：{e}");
                None
            }
        });
        Box::new(DellSensors {
            awcc,
            max_rpm: HashMap::new(),
        })
    }
}

/// 戴尔的只读传感器：独立的 AWCC 连接，只读温度和风扇。
struct DellSensors {
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
            cpu_temp: sensors.first().and_then(|&s| awcc.temperature(s)),
            gpu_temp: need
                .gpu_temp
                .then(|| sensors.get(1).and_then(|&s| awcc.temperature(s)))
                .flatten(),
            fans,
        }
    }
}

/// 百分比换算成 BIOS 的加速值（0～255）。
fn boost_value(percent: u8) -> u8 {
    (u32::from(percent.min(100)) * 255 / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(original: Option<u8>, g_mode: bool) -> DellG15 {
        DellG15 {
            name: String::new(),
            awcc: None,
            original_mode: original,
            g_mode,
        }
    }

    #[test]
    fn boost_percent_maps_onto_the_bios_range() {
        assert_eq!(boost_value(0), 0);
        assert_eq!(boost_value(50), 127);
        assert_eq!(boost_value(100), 255);
        assert_eq!(boost_value(250), 255);
    }

    #[test]
    fn matches_g15_only() {
        let g15 = BiosInfo {
            manufacturer: "Dell Inc.".into(),
            product: "Dell G15 5520".into(),
        };
        assert!(DellG15::matches(&g15));
        let xps = BiosInfo {
            manufacturer: "Dell Inc.".into(),
            product: "XPS 15 9520".into(),
        };
        assert!(!DellG15::matches(&xps));
    }

    #[test]
    fn balanced_returns_to_the_users_own_mode() {
        let p = profile(Some(0xA5), true);
        assert_eq!(p.target(Mode::Balanced, false), Thermal::Mode(0xA5));
        assert_eq!(p.target(Mode::Full, false), Thermal::Mode(PERFORMANCE));
        assert_eq!(p.target(Mode::Full, true), Thermal::Mode(0xA5));
        assert_eq!(p.target(Mode::Quiet, true), Thermal::Mode(QUIET));
    }

    #[test]
    fn vendor_mode_is_g_mode_when_the_bios_has_the_switch() {
        assert_eq!(
            profile(None, true).target(Mode::Vendor, false),
            Thermal::GMode
        );
        // 没有 G 模式开关：按全力处理；用电池：回到原模式。
        assert_eq!(
            profile(None, false).target(Mode::Vendor, false),
            Thermal::Mode(PERFORMANCE)
        );
        assert_eq!(
            profile(Some(0xA0), true).target(Mode::Vendor, true),
            Thermal::Mode(0xA0)
        );
    }

    #[test]
    fn without_bios_interface_there_is_no_thermal_control() {
        let mut p = profile(None, false);
        assert!(!p.thermal_control());
        assert_eq!(p.vendor_mode(), None);
        assert!(p.apply_thermal_mode(Mode::Full, false).is_ok());
    }
}
