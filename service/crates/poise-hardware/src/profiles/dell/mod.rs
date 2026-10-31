//! 戴尔 / 外星人档案（G 系列、Alienware 等带 AWCC 接口的机型）：通过 BIOS 的 AWCC 接口切换散热模式、
//! 风扇加速、G 模式；温度和风扇由独立连接的只读传感器读取。是否适用看 BIOS 有没有这个接口，不看型号。
//!
//! 散热模式只用 BIOS 自己列出的取值（AWCC 的 0xA0 一组：A0 均衡、A1 偏性能、A3 安静、A4 性能），
//! 并且均衡、安静和一种性能模式都在列表里才启用切换；
//! "均衡"不强加任何模式，而是回到用户原来的模式（Poise 启动时 BIOS 所处的模式，可以是 G 模式）。
//! G 模式（机型极限模式）需要 BIOS 提供 G 模式开关；离开 G 模式时先关开关，再切到目标模式。
//!
//! 原来的模式在换掉之前记在数据目录里（见 [`Takeover`]）：异常退出后下次启动先还原（记录里的取值不在
//! BIOS 列出的模式里时不用）。Poise 运行期间用户在 AWCC 里换了模式（和 Poise 上次写的不同），以用户的为准，
//! 当作新的原来的模式；刚换了供电时不算（可能是 BIOS 自己换的）。Poise 退出后用户自己换的模式，服务停止时
//! 不再改回。
//!
//! [`thermal`]：散热模式的取值与切换；[`sensors`]：只读传感器。

mod sensors;
mod thermal;

use std::path::Path;

use poise_core::prefs::Mode;

use super::awcc::Awcc;
use super::awcc::G_MODE;
use super::bios::BiosInfo;
use super::takeover::Takeover;
use super::{MachineProfile, MachineSensors};
use sensors::DellSensors;
use thermal::{BALANCED, BALANCED_PERFORMANCE, BiosState, PERFORMANCE, QUIET, Thermal};

pub struct Dell {
    name: String,
    awcc: Option<Awcc>,
    /// "全力"用的散热模式：BIOS 列出的最强的性能模式。
    performance: Option<u8>,
    /// 用户原来的散热模式，"均衡"和还原时回到它。读不到 BIOS 时为空，第一次切换前再读。
    original: Option<Thermal>,
    /// Poise 上次写入的（用来发现用户在 AWCC 里换了模式）；为空表示没有接管。
    wrote: Option<Thermal>,
    /// 上次切换时是否用电池（供电变了时 BIOS 自己换的模式不当作用户换的）。
    on_battery: Option<bool>,
    takeover: Takeover,
    /// BIOS 有 G 模式开关。
    g_mode: bool,
}

impl Dell {
    /// 戴尔或外星人的机器（再看 BIOS 有没有 AWCC 接口，见 [`Dell::open`]）。
    pub fn matches(bios: &BiosInfo) -> bool {
        let maker = bios.manufacturer.to_lowercase();
        maker.starts_with("dell") || maker.starts_with("alienware")
    }

    /// 连接 AWCC 接口；没有这个接口（例如商务本）时返回 None，用通用档案。
    pub fn open(bios: &BiosInfo, data_dir: &Path) -> Option<Self> {
        let awcc = match Awcc::open() {
            Ok(a) => {
                log::info!("AWCC 接口：{:?}", a.layout());
                a
            }
            Err(e) => {
                log::info!("AWCC 接口不可用（{e}），只使用通用能力");
                return None;
            }
        };
        let performance = [PERFORMANCE, BALANCED_PERFORMANCE]
            .into_iter()
            .find(|m| awcc.layout().power_modes.contains(m));
        let g_mode = awcc.g_mode().is_some();
        let (takeover, leftover) = Takeover::open(data_dir);
        let current = BiosState::read(&awcc).thermal();
        let listed = |raw: u8| {
            if raw == G_MODE {
                g_mode
            } else {
                awcc.layout().power_modes.contains(&raw)
            }
        };
        let leftover = match leftover {
            Some(raw) if listed(raw) => Some(raw),
            Some(raw) => {
                log::warn!("记录里原来的散热模式 {raw:#x} 不在 BIOS 列出的模式里，不用它");
                None
            }
            None => None,
        };
        let mut dell = Self {
            name: bios.product.clone(),
            awcc: Some(awcc),
            performance,
            original: current,
            wrote: None,
            on_battery: None,
            takeover,
            g_mode,
        };
        match leftover {
            // 上次换了散热模式、没来得及还原（异常退出）：先还原，不把留下的模式当成用户原来的。
            // 读不到此刻的模式时先不还原，记录留着，下次切换时照样以它为原来的模式。
            Some(raw) => {
                dell.original = Some(Thermal::from_raw(raw));
                dell.wrote = current;
                log::info!("上次退出时没有还原散热模式，还原为 {raw:#x}");
                dell.restore_thermal_mode();
            }
            None => dell.takeover.release(),
        }
        Some(dell)
    }

    /// 用户原来的散热模式；还没读到时按均衡。
    fn original(&self) -> Thermal {
        self.original.unwrap_or(Thermal::Mode(BALANCED))
    }
}

impl MachineProfile for Dell {
    fn name(&self) -> &str {
        &self.name
    }

    fn thermal_control(&self) -> bool {
        self.performance.is_some()
            && self.awcc.as_ref().is_some_and(|a| {
                let modes = &a.layout().power_modes;
                modes.contains(&BALANCED) && modes.contains(&QUIET)
            })
    }

    fn vendor_mode(&self) -> Option<&'static str> {
        (self.thermal_control() && self.g_mode).then_some("dell_g_mode")
    }

    fn apply_thermal_mode(&mut self, mode: Mode, on_battery: bool) -> Result<bool, String> {
        if !self.thermal_control() {
            return Ok(false);
        }
        let same_source = self.on_battery.is_none_or(|b| b == on_battery);
        let switched = self.switch(|d| d.target(mode, on_battery), same_source);
        // 切换成功后才记下供电：失败后再试时，BIOS 因换电源自己改的模式不会被当成用户换的。
        if switched.is_ok() {
            self.on_battery = Some(on_battery);
        }
        switched
    }

    fn recheck_thermal_mode(&mut self) -> Result<bool, String> {
        if !self.thermal_control() {
            return Ok(false);
        }
        self.recheck()
    }

    fn restore_thermal_mode(&mut self) {
        // 没有接管（还没切换过、已经还原过）：不碰，Poise 退出期间用户自己换的模式留着。
        if !self.thermal_control() || self.wrote.is_none() {
            return;
        }
        match self.switch(Self::original, true) {
            // 下次接管时重新读 BIOS（这期间用户可能在 AWCC 里换了模式）。
            Ok(_) => {
                self.wrote = None;
                self.on_battery = None;
                self.original = None;
                self.takeover.release();
            }
            Err(e) => log::warn!("还原散热模式失败：{e}"),
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
        Box::new(DellSensors::new(awcc))
    }
}

/// 百分比换算成 BIOS 的加速值（0～255）。
fn boost_value(percent: u8) -> u8 {
    (u32::from(percent.min(100)) * 255 / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(original: Option<u8>, g_mode: bool) -> Dell {
        let dir = std::env::temp_dir().join(format!("poise-dell-{}", std::process::id()));
        Dell {
            name: String::new(),
            awcc: None,
            performance: Some(BALANCED_PERFORMANCE),
            original: Some(Thermal::Mode(original.unwrap_or(BALANCED))),
            wrote: None,
            on_battery: None,
            takeover: Takeover::open(&dir).0,
            g_mode,
        }
    }

    #[test]
    fn the_users_own_g_mode_is_their_original_mode() {
        let mut p = profile(None, true);
        p.original = Some(Thermal::from_raw(G_MODE));
        assert_eq!(p.original, Some(Thermal::GMode));
        assert_eq!(p.original().raw(), G_MODE);
        assert_eq!(p.target(Mode::Balanced, false), Thermal::GMode);
        assert_eq!(p.target(Mode::Quiet, false), Thermal::Mode(QUIET));
    }

    #[test]
    fn the_same_target_as_last_time_does_not_call_the_bios() {
        // 没有 AWCC 连接：只要调用机型接口就会失败。
        let mut p = profile(Some(0xA0), false);
        p.wrote = Some(Thermal::Mode(0xA0));
        assert_eq!(
            p.switch(|d| d.target(Mode::Balanced, true), true),
            Ok(false)
        );
        assert!(p.switch(|d| d.target(Mode::Quiet, false), true).is_err());
        // 没有接管时还原不调用机型接口。
        p.wrote = None;
        p.restore_thermal_mode();
    }

    #[test]
    fn boost_percent_maps_onto_the_bios_range() {
        assert_eq!(boost_value(0), 0);
        assert_eq!(boost_value(50), 127);
        assert_eq!(boost_value(100), 255);
        assert_eq!(boost_value(250), 255);
    }

    #[test]
    fn dell_and_alienware_machines_are_probed_for_the_bios_interface() {
        let bios = |maker: &str| BiosInfo {
            manufacturer: maker.into(),
            product: String::new(),
        };
        assert!(Dell::matches(&bios("Dell Inc.")));
        assert!(Dell::matches(&bios("Alienware")));
        assert!(!Dell::matches(&bios("LENOVO")));
    }

    #[test]
    fn full_power_uses_the_strongest_listed_performance_mode() {
        let mut p = profile(Some(0xA0), false);
        p.performance = Some(PERFORMANCE);
        assert_eq!(p.target(Mode::Full, false), Thermal::Mode(PERFORMANCE));
    }

    #[test]
    fn balanced_returns_to_the_users_own_mode() {
        let p = profile(Some(0xA5), true);
        assert_eq!(p.target(Mode::Balanced, false), Thermal::Mode(0xA5));
        assert_eq!(
            p.target(Mode::Full, false),
            Thermal::Mode(BALANCED_PERFORMANCE)
        );
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
            Thermal::Mode(BALANCED_PERFORMANCE)
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
