//! Dell G15 系列档案：通过 BIOS 的 AWCC 接口读温度和风扇、切换散热模式。
//!
//! 散热模式只用 BIOS 自己列出的取值，并且三种目标模式都在列表里才启用切换；
//! "均衡"不强加任何模式，而是回到 Poise 启动时 BIOS 所处的模式（用户在 AWCC 里的选择）。

use poise_core::prefs::Mode;

use super::awcc::Awcc;
use super::bios::BiosInfo;
use super::{MachineProfile, MachineReading};

/// G15 5520 BIOS 列出的散热模式（实测：A0、A1、A3、A5）。
const BALANCED: u8 = 0xA0;
const PERFORMANCE: u8 = 0xA1;
const QUIET: u8 = 0xA3;

pub struct DellG15 {
    name: String,
    awcc: Option<Awcc>,
    /// Poise 启动时 BIOS 的散热模式，"均衡"和服务停止时回到它。
    original_mode: Option<u8>,
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
        let original_mode = awcc.as_ref().and_then(Awcc::power_mode);
        Self {
            name: bios.product.clone(),
            awcc,
            original_mode,
        }
    }

    fn target_mode(&self, mode: Mode, on_battery: bool) -> u8 {
        match mode {
            Mode::Quiet => QUIET,
            // 用电池时"全力"不加大散热功耗，只靠电源计划参数。
            Mode::Full if !on_battery => PERFORMANCE,
            _ => self.original_mode.unwrap_or(BALANCED),
        }
    }

    fn set_mode(&self, target: u8) -> Result<(), String> {
        let awcc = self.awcc.as_ref().ok_or("AWCC 接口不可用")?;
        if awcc.power_mode() == Some(target) {
            return Ok(());
        }
        awcc.set_power_mode(target)?;
        log::info!("散热模式：{target:#x}");
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

    fn apply_thermal_mode(&mut self, mode: Mode, on_battery: bool) -> Result<(), String> {
        if !self.thermal_control() {
            return Ok(());
        }
        self.set_mode(self.target_mode(mode, on_battery))
    }

    fn restore_thermal_mode(&mut self) {
        if self.thermal_control()
            && let Some(original) = self.original_mode
            && let Err(e) = self.set_mode(original)
        {
            log::warn!("还原散热模式失败：{e}");
        }
    }

    fn read(&mut self, include_fans: bool) -> MachineReading {
        let Some(awcc) = &self.awcc else {
            return MachineReading::default();
        };
        // BIOS 报告的传感器顺序：CPU、显卡。
        let sensors = &awcc.layout().sensors;
        MachineReading {
            cpu_temp: sensors.first().and_then(|&s| awcc.temperature(s)),
            gpu_temp: sensors.get(1).and_then(|&s| awcc.temperature(s)),
            fans: if include_fans {
                awcc.fans()
            } else {
                Vec::new()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let p = DellG15 {
            name: String::new(),
            awcc: None,
            original_mode: Some(0xA5),
        };
        assert_eq!(p.target_mode(Mode::Balanced, false), 0xA5);
        assert_eq!(p.target_mode(Mode::Full, false), PERFORMANCE);
        assert_eq!(p.target_mode(Mode::Full, true), 0xA5);
        assert_eq!(p.target_mode(Mode::Quiet, true), QUIET);
    }

    #[test]
    fn without_bios_interface_there_is_no_thermal_control() {
        let mut p = DellG15 {
            name: String::new(),
            awcc: None,
            original_mode: None,
        };
        assert!(!p.thermal_control());
        assert!(p.apply_thermal_mode(Mode::Full, false).is_ok());
    }
}
