//! 硬件模式：把"实际生效的模式"落到电源计划与散热上，并在过热时自动退出全力。

use poise_core::diagnose::{Health, HealthCode, Level};
use poise_core::model::Millis;
use poise_core::prefs::Mode;
use poise_platform::observe::system;

use super::Engine;

impl Engine {
    /// 应用电源/散热模式。温度过高时自动退出全力。观察模式下不写入。
    pub(super) fn apply_hardware_mode(&mut self, effective: Mode, health: &Health, now: Millis) {
        if self.cfg.dry_run {
            return;
        }
        let overheating = health.code == HealthCode::Hot && health.level == Level::Bad;
        let mode = if overheating && effective == Mode::Full {
            Mode::Balanced
        } else {
            effective
        };
        let on_battery = system::on_battery();
        if !self.hardware.needs_apply(mode, on_battery) {
            return;
        }
        let Self {
            hardware, changes, ..
        } = self;
        match changes.edit(|s, j| hardware.apply_mode(mode, on_battery, s, j, now)) {
            Ok(()) => log::info!("模式切换为 {mode:?}（电池：{on_battery}）"),
            Err(e) => log::warn!("切换模式失败：{e}"),
        }
    }
}
