//! 用户在界面上改的设置：模式、风扇加速、游戏时的模式、智能调度、自动清理。改完交给后台线程保存，
//! 需要时立即生效（重新调度一轮、写入风扇加速）。

use poise_core::prefs::{GameMode, Mode};
use poise_protocol::{ErrorCode, Event};

use super::Engine;
use super::error::RequestError;

impl Engine {
    pub(super) fn save_settings(&self) {
        self.settings_file.save_later(&self.settings, &self.files);
    }

    pub(super) fn set_mode(&mut self, mode: Mode) -> Result<(), RequestError> {
        if mode == Mode::Vendor && !self.vendor_available() {
            return Err(RequestError::new(
                ErrorCode::InvalidRequest,
                "这台电脑没有机型极限模式",
            ));
        }
        self.settings.mode = mode;
        self.save_settings();
        self.reschedule_now();
        Ok(())
    }

    pub(super) fn set_fan_boost(&mut self, percent: u8) {
        self.settings.fan_boost = percent.min(100);
        self.save_settings();
        // Poise 暂停时只保存，恢复时再落到硬件。
        if !self.paused {
            self.apply_fan_boost();
        }
    }

    pub(super) fn set_game_mode(&mut self, mode: GameMode) {
        self.settings.game_mode = mode;
        self.save_settings();
        // 正在玩游戏时立即换到新的目标模式。
        self.reschedule_now();
    }

    pub(super) fn set_smart_schedule(&mut self, enabled: bool) {
        if self.settings.smart_schedule != enabled {
            self.logbook.event(Event::SmartSchedule { enabled });
        }
        self.settings.smart_schedule = enabled;
        self.save_settings();
        // 关闭时立即撤销全部进程调整，打开时立即按类别调整。
        self.reschedule_now();
    }

    pub(super) fn set_auto_clean(&mut self, enabled: bool) {
        self.settings.auto_clean = enabled;
        self.save_settings();
    }
}
