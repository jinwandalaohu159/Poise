//! 把用户选择的模式换算成具体模式："自动"按负载、空闲、供电决定；
//! 机型不支持极限模式时按全力处理。游戏时的目标模式也在这里换算。

use poise_core::auto_mode::AutoInput;
use poise_core::classify::Classification;
use poise_core::fast_hash::HashMap;
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::prefs::Mode;

use super::Engine;

impl Engine {
    /// 此刻应使用的模式（不含游戏时的切换）。
    pub(super) fn resolve_user_mode(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        now: Millis,
    ) -> Mode {
        match self.settings.mode {
            Mode::Auto => {
                let input = AutoInput {
                    now,
                    on_battery: snapshot.system.on_battery,
                    user_idle_ms: self.session.current(now).idle_ms,
                    system_cpu_pct: self.activity.system_cpu_pct(),
                    foreground_cores: self.foreground_cores(snapshot, classes),
                };
                self.auto_mode.decide(&input)
            }
            Mode::Vendor if !self.vendor_available() => Mode::Full,
            mode => mode,
        }
    }

    /// 玩游戏时切换到的模式；None 表示不切换。
    pub(super) fn game_target(&self) -> Option<Mode> {
        self.settings.game_mode.target(self.vendor_available())
    }

    pub(super) fn vendor_available(&self) -> bool {
        self.hardware.vendor_mode().is_some()
    }

    /// 前台程序（整个进程树）占用的核心数。
    fn foreground_cores(&self, snapshot: &Snapshot, classes: &HashMap<Pid, Classification>) -> f32 {
        snapshot
            .processes
            .iter()
            .filter(|p| {
                classes
                    .get(&p.facts.key.pid)
                    .is_some_and(|c| c.in_foreground_tree)
            })
            .map(|p| self.activity.usage(&p.facts.key).core_pct)
            .sum::<f32>()
            / 100.0
    }
}
