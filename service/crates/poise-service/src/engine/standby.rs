//! 熄屏与待机。现代待机（S0 低功耗待机，合盖、按电源键）不会把服务停下：先熄屏，桌面程序（托盘代理、
//! 界面）随后被系统冻结，服务照常运行。没有这里的处理，托盘代理 20 秒报不上来就会被当成 Poise 退出了，
//! 每次合盖都撤销一切、开盖再重新接管。传统睡眠、休眠另有挂起与唤醒的通知；两种都收不到时（控制台方式），
//! 靠两轮采样之间隔了很久发现（见 `cycle`）。
//!
//! 熄屏或挂起期间：托盘代理的上报按住不过期（它真的退出了照样立即暂停，见 `SessionTracker::hold`），
//! 采样放慢，传感器读得很少。亮屏、醒来时：放开上报，待机得够久的话重新确认硬件状态（固件可能在待机时
//! 改回了散热模式），立即调度一轮。

use poise_core::model::Millis;

use super::Engine;

/// 熄屏这么久以上才算待机过，醒来时重新确认硬件状态（只是熄屏一下、马上又亮起来时不必）。
const RECHECK_AFTER_MS: Millis = 15_000;

/// 系统的电源变化（服务从服务管理器收到，见 `host`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerChange {
    /// 屏幕熄了（现代待机最先到来的信号；只是到时间熄屏也一样：都没人在看桌面）。
    DisplayOff,
    /// 屏幕亮了（变暗也算亮着）。
    DisplayOn,
    /// 传统睡眠、休眠：即将挂起。
    Suspending,
    /// 从挂起中醒来。
    Resumed,
}

/// 熄屏、挂起的状态。
#[derive(Debug, Default)]
pub(super) struct Standby {
    display_off: bool,
    suspended: bool,
    /// 开始熄屏或挂起的时刻。
    since: Option<Millis>,
}

/// 一次电源变化带来的转变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Entered,
    /// 离开待机；带着待机了多久。
    Left(Millis),
    None,
}

impl Standby {
    /// 熄屏或挂起着。
    pub(super) fn active(&self) -> bool {
        self.since.is_some()
    }

    fn apply(&mut self, change: PowerChange, now: Millis) -> Turn {
        match change {
            PowerChange::DisplayOff => self.display_off = true,
            PowerChange::DisplayOn => self.display_off = false,
            PowerChange::Suspending => self.suspended = true,
            // 醒来时屏幕也亮了的话随后另有通知；没亮（定时唤醒做维护）时仍按熄屏算。
            PowerChange::Resumed => self.suspended = false,
        }
        let now_active = self.display_off || self.suspended;
        match (self.since, now_active) {
            (None, true) => {
                self.since = Some(now);
                Turn::Entered
            }
            (Some(since), false) => {
                self.since = None;
                Turn::Left(now.saturating_sub(since))
            }
            _ => Turn::None,
        }
    }
}

impl Engine {
    /// 系统的电源变化（见模块说明）。
    pub fn on_power(&mut self, change: PowerChange) {
        if self.stopped {
            return;
        }
        let now = self.now();
        match self.standby.apply(change, now) {
            Turn::Entered => {
                self.session.hold(now);
                log::info!("{change:?}：进入待机，托盘代理没上报不当成退出");
            }
            Turn::Left(lasted) => {
                self.session.release(now);
                // 待机期间采样隔得很久，不当成睡眠（见 `cycle`）；这里已经处理过醒来。
                self.last_tick_at = None;
                if lasted >= RECHECK_AFTER_MS {
                    self.hardware.resumed();
                }
                self.last_change_at = now;
                self.answers.invalidate();
                log::info!("{change:?}：离开待机（{} 秒）", lasted / 1000);
            }
            Turn::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standby_lasts_while_the_screen_is_off_or_the_system_is_suspended() {
        let mut s = Standby::default();
        assert_eq!(s.apply(PowerChange::DisplayOn, 0), Turn::None);
        assert_eq!(s.apply(PowerChange::DisplayOff, 1_000), Turn::Entered);
        assert_eq!(s.apply(PowerChange::Suspending, 2_000), Turn::None);
        // 醒来了但屏幕还没亮（定时唤醒）：仍在待机。
        assert_eq!(s.apply(PowerChange::Resumed, 60_000), Turn::None);
        assert!(s.active());
        assert_eq!(s.apply(PowerChange::DisplayOn, 61_000), Turn::Left(60_000));
        assert!(!s.active());
        // 只有挂起、醒来（没有屏幕通知）。
        assert_eq!(s.apply(PowerChange::Suspending, 70_000), Turn::Entered);
        assert_eq!(s.apply(PowerChange::Resumed, 80_000), Turn::Left(10_000));
    }
}
