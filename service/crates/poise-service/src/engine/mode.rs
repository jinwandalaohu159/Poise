//! 硬件模式：把"实际生效的模式"落到电源计划与散热上，并在过热时自动退出全力和极限模式；风扇加速按用户设置。
//! 观察模式下不写入，只在将要变化时记一条日志。

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::optimize::SettingStore;
use poise_core::prefs::Mode;
use poise_hardware::Hardware;
use poise_platform::observe::system;

use super::Engine;
use super::changes::ChangeError;

/// 过热退出性能模式后，至少保持这么久（并且温度降回正常）才恢复。
const OVERHEAT_HOLD_MS: Millis = 5 * 60_000;

impl Engine {
    /// 应用电源/散热模式，返回实际落到硬件上的模式（界面显示它）。
    /// 温度危险时自动退出全力和极限模式（见 [`poise_core::diagnose::OverheatGuard`]）。观察模式下不写入。
    fn apply_hardware_mode(&mut self, effective: Mode, now: Millis) -> Mode {
        let overheating = self.overheat.update(self.last_heat, now, OVERHEAT_HOLD_MS);
        let mode = if overheating && effective.is_performance() {
            Mode::Balanced
        } else {
            effective
        };
        let on_battery = system::on_battery();
        if self.cfg.dry_run {
            if self.observed.mode != Some((mode, on_battery)) {
                self.observed.mode = Some((mode, on_battery));
                log::info!(
                    "[观察] 模式将切换为 {mode:?}（电池：{on_battery}），电源计划与散热模式不写入"
                );
            }
            return mode;
        }
        if !self.hardware.needs_apply(mode, on_battery) {
            return mode;
        }
        match self.edit_hardware(|h, s, j| h.apply_mode(mode, on_battery, s, j, now)) {
            Ok(()) => log::info!("模式切换为 {mode:?}（电池：{on_battery}）"),
            Err(e) => log::warn!("切换模式失败：{e}"),
        }
        mode
    }

    /// 服务停止：硬件模式回到 Poise 接管之前的状态。
    pub(super) fn restore_hardware(&mut self) {
        let now = self.now();
        let _ = self.edit_hardware(|h, s, j| {
            h.restore(s, j, now);
            Ok(())
        });
    }

    /// 通过变更日志修改硬件设置（同时借用硬件和变更日志）。
    fn edit_hardware(
        &mut self,
        f: impl FnOnce(&mut Hardware, &dyn SettingStore, &mut Journal) -> Result<(), JournalError>,
    ) -> Result<(), ChangeError> {
        let Self {
            hardware, changes, ..
        } = self;
        changes.edit(|s, j| f(hardware, s, j))
    }

    /// 把实际模式和风扇加速落到硬件，返回落到硬件上的模式（过热时可能被降级）。
    pub(super) fn apply_modes(&mut self, effective: Mode, now: Millis) -> Mode {
        let mode = self.apply_hardware_mode(effective, now);
        self.apply_fan_boost();
        mode
    }

    /// 风扇加速按用户设置。与上次写入的相同时硬件层不重复写。
    pub(super) fn apply_fan_boost(&mut self) {
        let percent = self.settings.fan_boost;
        if self.cfg.dry_run {
            if self.observed.fan_boost != Some(percent) {
                self.observed.fan_boost = Some(percent);
                log::info!("[观察] 风扇加速将设为 {percent}%，不写入");
            }
            return;
        }
        self.hardware.apply_fan_boost(percent);
    }
}

/// 观察模式下最近一次"将要写入"的硬件状态：只在变化时记日志。
#[derive(Default)]
pub(super) struct Observed {
    mode: Option<(Mode, bool)>,
    fan_boost: Option<u8>,
}
