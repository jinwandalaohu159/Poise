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
use poise_hardware::{Applied, Hardware};
use poise_protocol::Event;

use super::Engine;
use super::changes::ChangeError;

impl Engine {
    /// 应用电源/散热模式，返回实际落到硬件上的模式（界面显示它）。观察模式下不写入。
    fn apply_hardware_mode(&mut self, mode: Mode, on_battery: bool, now: Millis) -> Mode {
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
            self.mode_attempts.settled();
            return mode;
        }
        if !self.mode_attempts.due((mode, on_battery), now) {
            return mode;
        }
        let started = Instant::now();
        let result = self.edit_hardware(|h, s, j| h.apply_mode(mode, on_battery, s, j, now));
        self.mode_attempts.record((mode, on_battery), &result, now);
        match result {
            // 第一次接管，原值还在预读：读好后再写（界面先显示要切换到的模式；这期间采样勤一些，见 `cadence`）。
            Ok(Applied::Unchanged | Applied::Waiting) => {}
            // 记下要写入的设置和调度为它停了多久（真正写进电源服务在硬件执行线程里，慢时另记日志）。
            // 不读回确认：电源服务这时正忙着让新设置生效，读一次要几百毫秒到两秒多；
            // 真被别处改掉的由电源守护发现、改回。
            Ok(Applied::Written) => {
                let detail = format!(
                    "{}（记录 {} 毫秒）",
                    self.hardware.describe_power(mode, on_battery),
                    started.elapsed().as_millis()
                );
                self.logbook
                    .event_with(Event::ModeChanged { mode, on_battery }, detail);
            }
            Err(e) => self
                .logbook
                .event_with(Event::ModeFailed { mode }, e.to_string()),
        }
        mode
    }

    /// 硬件模式回到 Poise 接管之前的状态（Poise 退出、服务停止），不等待（要等时见 [`Engine::settle_hardware`]）。
    pub(super) fn restore_hardware(&mut self) {
        let _ = self.edit_hardware(|h, s, j| {
            h.restore(s, j);
            Ok(())
        });
    }

    /// 等交给执行线程的还原做完（排着的电源设置写完、散热模式还原），之后落盘的日志不再带上撤销了的条目。
    pub(super) fn settle_hardware(&mut self) {
        self.hardware.settle();
        self.changes.settle();
    }

    /// 通过变更日志修改硬件设置（同时借用硬件和变更日志）。
    fn edit_hardware<T>(
        &mut self,
        f: impl FnOnce(&mut Hardware, &dyn SettingStore, &mut Journal) -> Result<T, JournalError>,
    ) -> Result<T, ChangeError> {
        let Self {
            hardware, changes, ..
        } = self;
        changes.edit(|s, j| f(hardware, s, j))
    }

    /// 把实际模式和风扇加速落到硬件，返回落到硬件上的模式。`on_battery` 取这一轮采样时的供电状态。
    pub(super) fn apply_modes(&mut self, effective: Mode, on_battery: bool, now: Millis) -> Mode {
        let mode = self.apply_hardware_mode(effective, on_battery, now);
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
