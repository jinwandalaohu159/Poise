//! 调度策略：根据一个进程的情况，决定它"应该"处于什么状态。纯函数，不记状态。

use crate::action::{CoreSet, EcoQos};
use crate::activity::Usage;
use crate::classify::Classification;
use crate::model::PriorityClass;
use crate::prefs::{AppPreference, Mode};

/// 一个进程期望的调度状态。`priority` 为 None 表示保持原始优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Desired {
    pub priority: Option<PriorityClass>,
    pub eco: EcoQos,
    pub cores: CoreSet,
}

impl Desired {
    pub const NEUTRAL: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::All,
    };

    const GAME: Self = Self {
        priority: Some(PriorityClass::High),
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    const FOREGROUND: Self = Self {
        priority: Some(PriorityClass::AboveNormal),
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    const FAST: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    const KEEP_AWAY: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    const YIELD_TO_E: Self = Self {
        priority: None,
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    const YIELD_SOFT: Self = Self {
        priority: None,
        eco: EcoQos::On,
        cores: CoreSet::All,
    };

    pub fn is_neutral(&self) -> bool {
        *self == Self::NEUTRAL
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig {
    /// 后台进程超过这个单核占用才需要让路（空闲进程不值得调整）。
    pub yield_min_core_pct: f32,
    /// 已经在让路的进程，降到这个值以下才放开（滞回，避免来回切换）。
    pub yield_keep_core_pct: f32,
    /// 前台进程树超过这个单核占用视为"前台忙"。
    pub foreground_busy_core_pct: f32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            yield_min_core_pct: 5.0,
            yield_keep_core_pct: 1.0,
            foreground_busy_core_pct: 25.0,
        }
    }
}

/// 全局情况：对所有进程相同。
#[derive(Debug, Clone, Copy)]
pub struct PolicyContext {
    pub mode: Mode,
    pub foreground_busy: bool,
}

/// 单个进程的情况。
#[derive(Debug, Clone, Copy)]
pub struct ProcessView<'a> {
    pub class: &'a Classification,
    pub usage: Usage,
    /// 游戏正在运行（包括切到后台的游戏）。
    pub active_game: bool,
    /// 刚失去前台不久（防抖期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
    /// 当前已经处于让路状态。
    pub yielding: bool,
}

pub fn decide(cfg: &PolicyConfig, ctx: &PolicyContext, v: &ProcessView) -> Desired {
    let c = v.class;
    if !c.is_schedulable() {
        return Desired::NEUTRAL;
    }
    if v.active_game {
        return Desired::GAME;
    }
    if c.in_foreground_tree || v.recently_foreground {
        return Desired::FOREGROUND;
    }
    match c.preference {
        AppPreference::AlwaysFast => return Desired::FAST,
        AppPreference::Background => return Desired::KEEP_AWAY,
        AppPreference::Auto | AppPreference::Game => {}
    }
    if v.boosted {
        return Desired::FAST;
    }
    if c.plays_audio {
        return Desired::NEUTRAL;
    }
    let threshold = if v.yielding {
        cfg.yield_keep_core_pct
    } else {
        cfg.yield_min_core_pct
    };
    if v.usage.core_pct < threshold {
        return Desired::NEUTRAL;
    }
    match ctx.mode {
        Mode::Quiet => Desired::YIELD_SOFT,
        Mode::Balanced if ctx.foreground_busy => Desired::YIELD_TO_E,
        Mode::Balanced => Desired::NEUTRAL,
        Mode::Full => Desired::YIELD_TO_E,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Role;

    fn class() -> Classification {
        Classification {
            role: Role::UserApp,
            root_pid: 1,
            in_foreground_tree: false,
            has_visible_window: false,
            tree_has_window: false,
            plays_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: None,
        }
    }

    fn view(c: &Classification, core_pct: f32) -> ProcessView<'_> {
        ProcessView {
            class: c,
            usage: Usage {
                core_pct,
                ..Default::default()
            },
            active_game: false,
            recently_foreground: false,
            boosted: false,
            yielding: false,
        }
    }

    fn ctx(mode: Mode, busy: bool) -> PolicyContext {
        PolicyContext {
            mode,
            foreground_busy: busy,
        }
    }

    fn d(c: &PolicyContext, v: &ProcessView) -> Desired {
        decide(&PolicyConfig::default(), c, v)
    }

    #[test]
    fn foreground_is_boosted_and_game_gets_high() {
        let fg = Classification {
            in_foreground_tree: true,
            ..class()
        };
        assert_eq!(
            d(&ctx(Mode::Balanced, false), &view(&fg, 0.0)),
            Desired::FOREGROUND
        );
        let c = class();
        let mut v = view(&c, 0.0);
        v.active_game = true;
        assert_eq!(d(&ctx(Mode::Balanced, false), &v), Desired::GAME);
    }

    #[test]
    fn balanced_yields_only_when_foreground_busy() {
        let c = class();
        assert_eq!(
            d(&ctx(Mode::Balanced, false), &view(&c, 30.0)),
            Desired::NEUTRAL
        );
        assert_eq!(
            d(&ctx(Mode::Balanced, true), &view(&c, 30.0)),
            Desired::YIELD_TO_E
        );
    }

    #[test]
    fn idle_background_is_left_alone() {
        let c = class();
        assert_eq!(d(&ctx(Mode::Full, true), &view(&c, 0.5)), Desired::NEUTRAL);
    }

    #[test]
    fn hysteresis_keeps_yielding_until_usage_drops_low() {
        let c = class();
        let mut v = view(&c, 3.0);
        assert_eq!(d(&ctx(Mode::Full, true), &v), Desired::NEUTRAL);
        v.yielding = true;
        assert_eq!(d(&ctx(Mode::Full, true), &v), Desired::YIELD_TO_E);
    }

    #[test]
    fn quiet_mode_uses_eco_without_pinning() {
        let c = class();
        assert_eq!(
            d(&ctx(Mode::Quiet, false), &view(&c, 30.0)),
            Desired::YIELD_SOFT
        );
    }

    #[test]
    fn preferences_and_boost_override_generic_rules() {
        let fast = Classification {
            preference: AppPreference::AlwaysFast,
            ..class()
        };
        assert_eq!(d(&ctx(Mode::Full, true), &view(&fast, 90.0)), Desired::FAST);
        let away = Classification {
            preference: AppPreference::Background,
            ..class()
        };
        assert_eq!(
            d(&ctx(Mode::Balanced, false), &view(&away, 0.0)),
            Desired::KEEP_AWAY
        );
        let c = class();
        let mut v = view(&c, 90.0);
        v.boosted = true;
        assert_eq!(d(&ctx(Mode::Full, true), &v), Desired::FAST);
    }

    #[test]
    fn audio_and_debounced_processes_are_protected() {
        let audio = Classification {
            plays_audio: true,
            ..class()
        };
        assert_eq!(
            d(&ctx(Mode::Full, true), &view(&audio, 50.0)),
            Desired::NEUTRAL
        );
        let c = class();
        let mut v = view(&c, 50.0);
        v.recently_foreground = true;
        assert_eq!(d(&ctx(Mode::Full, true), &v), Desired::FOREGROUND);
    }

    #[test]
    fn non_user_processes_stay_neutral() {
        let sys = Classification {
            role: Role::SystemOther,
            in_foreground_tree: true,
            ..class()
        };
        assert_eq!(
            d(&ctx(Mode::Full, true), &view(&sys, 90.0)),
            Desired::NEUTRAL
        );
    }
}
