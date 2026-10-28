//! 调度策略：根据一个进程的类别和前台/后台状态，决定它"应该"处于什么状态。纯函数，不记状态。
//!
//! | 类别 | 在后台 | 在前台 |
//! |---|---|---|
//! | 低优先级 | 小核 · 低于正常 · 节能 | 小核 · 低于正常 · 不节能 |
//! | 自动 | 小核 · 低于正常 · 节能 | 不限核心 · 高于正常 · 不节能 |
//! | 始终优先（含临时全速） | 不限核心 · 高于正常 · 不节能 | 同左 |
//! | 游戏 | 不限核心 · 高 · 不节能 | 同左 |
//!
//! 正在发声的程序在后台只挪到小核；白名单里的后台杂活只挪到小核并节能，不改优先级。

use crate::action::{CoreSet, EcoQos};
use crate::classify::Classification;
use crate::model::PriorityClass;
use crate::prefs::AppPreference;

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

    /// 前台程序、始终优先、临时全速。
    const RAISED: Self = Self {
        priority: Some(PriorityClass::AboveNormal),
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    /// 在后台时让出性能（自动、低优先级）。
    const YIELDED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    /// 低优先级程序到了前台：仍在小核，但不节能，界面照样能点。
    const LOW_IN_FOREGROUND: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Off,
        cores: CoreSet::Efficiency,
    };

    /// 在后台发声的程序：只挪到小核，不节能、不降优先级。
    const AUDIO_IN_BACKGROUND: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::Efficiency,
    };

    const CHORE: Self = Self {
        priority: None,
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    pub fn is_neutral(&self) -> bool {
        *self == Self::NEUTRAL
    }
}

/// 单个进程的情况。
#[derive(Debug, Clone, Copy)]
pub struct ProcessView<'a> {
    pub class: &'a Classification,
    /// 游戏正在运行（包括切到后台的游戏）。
    pub active_game: bool,
    /// 刚失去前台不久（防抖期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
}

/// 安全守卫（`safety::SafetyGuard::check_demote`）按同样的规则再把关一次：改这里时那边一起改。
pub fn decide(v: &ProcessView) -> Desired {
    let c = v.class;
    if c.is_chore() {
        return Desired::CHORE;
    }
    if !c.is_schedulable() {
        return Desired::NEUTRAL;
    }
    if v.active_game {
        return Desired::GAME;
    }
    let category = if v.boosted {
        AppPreference::AlwaysFast
    } else {
        c.preference
    };
    let foreground = c.in_foreground_tree || v.recently_foreground;
    match category {
        AppPreference::AlwaysFast => Desired::RAISED,
        AppPreference::Background if foreground => Desired::LOW_IN_FOREGROUND,
        AppPreference::Auto | AppPreference::Game if foreground => Desired::RAISED,
        _ if c.plays_audio => Desired::AUDIO_IN_BACKGROUND,
        _ => Desired::YIELDED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Role, ScheduleScope};

    fn class(preference: AppPreference, foreground: bool) -> Classification {
        Classification {
            role: Role::UserApp,
            root_pid: 1,
            in_foreground_tree: foreground,
            has_visible_window: false,
            tree_has_window: false,
            plays_audio: false,
            recently_active: false,
            preference,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: None,
            schedule: ScheduleScope::App,
        }
    }

    fn view(c: &Classification) -> ProcessView<'_> {
        ProcessView {
            class: c,
            active_game: false,
            recently_foreground: false,
            boosted: false,
        }
    }

    fn d(c: &Classification) -> Desired {
        decide(&view(c))
    }

    #[test]
    fn each_category_in_background_and_foreground() {
        use AppPreference::*;
        let cases = [
            (Background, false, Desired::YIELDED),
            (Background, true, Desired::LOW_IN_FOREGROUND),
            (Auto, false, Desired::YIELDED),
            (Auto, true, Desired::RAISED),
            (AlwaysFast, false, Desired::RAISED),
            (AlwaysFast, true, Desired::RAISED),
        ];
        for (pref, fg, want) in cases {
            assert_eq!(d(&class(pref, fg)), want, "{pref:?} 前台={fg}");
        }
    }

    #[test]
    fn games_get_high_even_in_background() {
        let c = class(AppPreference::Game, false);
        let mut v = view(&c);
        v.active_game = true;
        assert_eq!(decide(&v), Desired::GAME);
    }

    #[test]
    fn debounce_keeps_the_foreground_state() {
        let c = class(AppPreference::Auto, false);
        let mut v = view(&c);
        v.recently_foreground = true;
        assert_eq!(decide(&v), Desired::RAISED);
    }

    #[test]
    fn boost_overrides_the_category() {
        let c = class(AppPreference::Background, false);
        let mut v = view(&c);
        v.boosted = true;
        assert_eq!(decide(&v), Desired::RAISED);
    }

    #[test]
    fn audio_in_background_only_moves_cores() {
        let c = Classification {
            plays_audio: true,
            ..class(AppPreference::Auto, false)
        };
        assert_eq!(d(&c), Desired::AUDIO_IN_BACKGROUND);
        let fg = Classification {
            plays_audio: true,
            ..class(AppPreference::Auto, true)
        };
        assert_eq!(d(&fg), Desired::RAISED);
    }

    #[test]
    fn chores_move_to_small_cores_without_priority() {
        let c = Classification {
            role: Role::ServiceHost,
            schedule: ScheduleScope::Chore,
            ..class(AppPreference::Auto, false)
        };
        assert_eq!(d(&c), Desired::CHORE);
        assert_eq!(Desired::CHORE.priority, None);
    }

    #[test]
    fn excluded_processes_stay_neutral() {
        let c = Classification {
            role: Role::SystemOther,
            schedule: ScheduleScope::Excluded,
            ..class(AppPreference::Auto, true)
        };
        assert_eq!(d(&c), Desired::NEUTRAL);
    }
}
