//! 界面要的东西立即跟上，不等下一轮采样：
//!
//! - 界面刚打开：叫运行循环立即采一轮（进程、核心面板、走势、传感器的详细程度都马上跟上）；
//! - 传感器在两轮之间放出了新读数（界面刚打开时显卡、风扇读得比下一轮采样早）：界面要状态时直接换上。

use poise_core::model::Millis;
use poise_hardware::SensorLevel;

use super::Engine;

/// 离上一轮采样不到这么久就不再叫运行循环：窗口来回显示、隐藏时不连着采。
const WAKE_MIN_GAP_MS: Millis = 1_000;

impl Engine {
    /// 传感器在两轮采样之间放出了新读数（界面刚打开时，显卡、风扇读得比下一轮采样早）：换到手里的读数上，
    /// 界面要状态时立即看到，不等下一轮。只换读数，健康状态、走势照常在每轮里算。
    pub(super) fn take_fresh_reading(&mut self) -> bool {
        let generation = self.sensors.generation();
        if generation == self.reading_generation {
            return false;
        }
        let Some(view) = self.view.as_mut() else {
            return false;
        };
        self.reading_generation = generation;
        view.reading = self.sensors.latest();
        true
    }

    /// 界面刚打开：传感器立即按"界面显示着"的详细程度读（显卡、风扇），不等下一轮采样来告诉它；
    /// 运行循环也立即采一轮（离上一轮太近就不必，那一轮的快照还是新的）。
    pub(super) fn on_ui_shown(&mut self, now: Millis) {
        let (gaming, focused) = self
            .view
            .as_ref()
            .map_or((false, false), |v| (v.game.is_some(), v.focused));
        self.sensors
            .set_level(SensorLevel::choose(true, gaming, focused));
        self.wake(now);
    }

    /// 运行循环每轮之间等着；界面刚打开时叫它立即采一轮（进程、核心面板、走势、传感器的详细程度都马上
    /// 跟上）。离上一轮不到 [`WAKE_MIN_GAP_MS`] 就不叫：窗口来回显示、隐藏时不连着采。
    pub fn set_waker(&mut self, waker: impl Fn() + Send + 'static) {
        self.waker = Some(Box::new(waker));
    }

    fn wake(&self, now: Millis) {
        let recent = self
            .last_tick_at
            .is_some_and(|t| now.saturating_sub(t) < WAKE_MIN_GAP_MS);
        if let (Some(waker), false) = (&self.waker, recent) {
            waker();
        }
    }
}
