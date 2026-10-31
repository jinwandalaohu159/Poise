//! 戴尔的散热模式：BIOS 的取值、此刻的状态，以及按 Poise 的模式切换（见上级模块的说明）。

use poise_core::prefs::Mode;

use super::super::awcc::{Awcc, G_MODE};
use super::Dell;

/// BIOS 的散热模式取值（G15 5520 实测列出 A0、A1、A3、A5）。
pub(super) const BALANCED: u8 = 0xA0;
/// 偏性能（G 系列的"性能"）。
pub(super) const BALANCED_PERFORMANCE: u8 = 0xA1;
pub(super) const QUIET: u8 = 0xA3;
/// 性能（部分外星人机型才有）。
pub(super) const PERFORMANCE: u8 = 0xA4;

/// 散热目标：BIOS 散热模式，或 G 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Thermal {
    Mode(u8),
    GMode,
}

impl Thermal {
    /// 记录里的原始取值（G 模式记为 [`G_MODE`]）。
    pub(super) fn from_raw(raw: u8) -> Self {
        if raw == G_MODE {
            Self::GMode
        } else {
            Self::Mode(raw)
        }
    }

    pub(super) fn raw(self) -> u8 {
        match self {
            Self::Mode(m) => m,
            Self::GMode => G_MODE,
        }
    }
}

/// BIOS 此刻的状态（两次机型接口调用）。
pub(super) struct BiosState {
    g_on: bool,
    power: Option<u8>,
}

impl BiosState {
    pub(super) fn read(awcc: &Awcc) -> Self {
        Self {
            g_on: awcc.g_mode() == Some(true),
            power: awcc.power_mode(),
        }
    }

    pub(super) fn thermal(&self) -> Option<Thermal> {
        if self.g_on || self.power == Some(G_MODE) {
            Some(Thermal::GMode)
        } else {
            self.power.map(Thermal::Mode)
        }
    }
}

impl Dell {
    /// Poise 的模式对应的散热目标。
    pub(super) fn target(&self, mode: Mode, on_battery: bool) -> Thermal {
        match mode {
            Mode::Quiet => Thermal::Mode(QUIET),
            // 用电池时不加大散热功耗，只靠电源计划参数。
            Mode::Vendor if !on_battery && self.g_mode => Thermal::GMode,
            Mode::Full | Mode::Vendor if !on_battery => {
                Thermal::Mode(self.performance.unwrap_or(BALANCED_PERFORMANCE))
            }
            // G 模式不在 AWCC 的模式列表里：用户原来开着 G 模式时，"全力"以外都回到它。
            _ => self.original(),
        }
    }

    /// 切换到 `wanted` 算出的目标，返回是否写了 BIOS（写了会把风扇加速清零）。和上次写入的相同时不调用
    /// 机型接口（用户之后在 AWCC 里改的也不覆盖）；否则先看用户有没有在 AWCC 里换过模式（`adopt` 时，换过的
    /// 当作新的原来的模式），再按需写入。换掉原来的模式之前先记下（见 `Takeover`）：写到一半崩溃也能还原。
    pub(super) fn switch(
        &mut self,
        wanted: impl Fn(&Self) -> Thermal,
        adopt: bool,
    ) -> Result<bool, String> {
        if self.wrote.is_some_and(|w| w == wanted(self)) {
            return Ok(false);
        }
        let awcc = self.awcc.as_ref().ok_or("AWCC 接口不可用")?;
        let state = BiosState::read(awcc);
        let now = state.thermal();
        if let (Some(wrote), Some(now)) = (self.wrote, now)
            && now != wrote
            && adopt
        {
            log::info!(
                "散热模式在 Poise 之外被换成了 {:#x}，以它为原来的模式",
                now.raw()
            );
            self.original = Some(now);
        }
        if self.original.is_none() {
            self.original = now;
        }
        let original = self.original.ok_or("读不到 BIOS 此刻的散热模式")?;
        let target = wanted(self);
        if target != original {
            self.takeover.hold(original.raw());
        }
        let mut wrote_bios = false;
        match target {
            Thermal::GMode => {
                if now != Some(Thermal::GMode) {
                    awcc.set_g_mode(true)?;
                    wrote_bios = true;
                    log::info!("散热模式：G 模式");
                }
            }
            Thermal::Mode(mode) => {
                let mut power = state.power;
                if self.g_mode && state.g_on {
                    awcc.set_g_mode(false)?;
                    wrote_bios = true;
                    power = None;
                }
                if power != Some(mode) {
                    awcc.set_power_mode(mode)?;
                    wrote_bios = true;
                    log::info!("散热模式：{mode:#x}");
                }
            }
        }
        self.wrote = Some(target);
        if target == original {
            self.takeover.release();
        }
        Ok(wrote_bios)
    }
}
