//! 硬件模式：把"实际生效的模式"落到电源计划与散热上；风扇加速按用户设置。
//! 观察模式下不写入，只在将要变化时记一条日志。
//!
//! 不因为温度切换模式：处理器和显卡到了温度上限会自己降频保护（i7-12700H 这类笔记本处理器在高性能模式下
//! 本来就工作在 90～100°C）。以前温度一高就退回低功耗模式、5 分钟后再切回来，结果是玩游戏时反复切换、
//! 大部分时间跑在低功耗模式下。

use std::time::Instant;

use poise_core::journal::{Journal, JournalError};
use poise_core::model::Millis;
use poise_core::prefs::Mode;
use poise_core::setting::SettingStore;
use poise_hardware::Hardware;
use poise_platform::observe::system;

use super::Engine;
use super::changes::ChangeError;

impl Engine {
    /// 应用电源/散热模式，返回实际落到硬件上的模式（界面显示它）。观察模式下不写入。
    fn apply_hardware_mode(&mut self, mode: Mode, now: Millis) -> Mode {
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
        let started = Instant::now();
        let result = self.edit_hardware(|h, s, j| h.apply_mode(mode, on_battery, s, j, now));
        let written_in = started.elapsed();
        match result {
            // 读回实际的设置，确认真的写进去了。记下写入和读回各用了多久（切换期间调度在等它）。
            Ok(()) => {
                let settings = self
                    .hardware
                    .describe_power(&self.changes.store, on_battery);
                log::info!(
                    "模式切换为 {mode:?}（电池：{on_battery}）：{settings}（写入 {} 毫秒，读回 {} 毫秒）",
                    written_in.as_millis(),
                    (started.elapsed() - written_in).as_millis()
                );
            }
            Err(e) => log::warn!("切换模式失败：{e}"),
        }
        mode
    }

    /// 服务停止：硬件模式回到 Poise 接管之前的状态。
    pub(super) fn restore_hardware(&mut self) {
        let _ = self.edit_hardware(|h, s, j| {
            h.restore(s, j);
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

    /// 把实际模式和风扇加速落到硬件，返回落到硬件上的模式。
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
