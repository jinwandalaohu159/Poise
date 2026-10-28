//! 调度策略：根据一个进程的情况，决定它"应该"处于什么状态。纯函数，不记状态。
//!
//! 原则是**让路为主，基本不提速**：大幅提速会和声音、输入、系统组件抢资源，反而卡顿。Poise 主要把在后台
//! 持续占用处理器的程序挪开，让正在用的程序自然拿到资源。唯一的提速是用户标记为"游戏"的程序：
//! 提高一级（正常 → 高于正常），不会更高。
//!
//! 另外弥补 Windows 的短板：Windows 会自动给后台、最小化的程序降频、放宽计时器精度，
//! 游戏切出去、后台音乐和语音会因此卡顿。对这些程序明确关闭系统的自动节能——不限核心。
//!
//! | 情况 | 状态 |
//! |---|---|
//! | 用户标记为"游戏" | 优先级高一级 · 不受系统节能限速 |
//! | 正在玩的游戏、始终优先、临时全速、正在发声（整个进程树，停声后保持一会儿） | 不受系统节能限速 |
//! | 前台、刚用过的程序 | 不调整（Windows 本来就照顾前台） |
//! | 低优先级（用户指定） | 小核 · 低于正常 · 在后台节能 |
//! | 自动，在后台持续占用处理器 | 小核 · 低于正常 · 节能（保持到它回到前台、发声或退出） |
//! | 自动，在后台闲着 | 不调整（闲着的程序不和谁抢资源，不值得动它） |
//!
//! 系统进程不在调度范围内。

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

    /// 在后台让出性能。
    pub const YIELDED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
    };

    /// 用户标记的游戏：优先级提高一级（高于正常，见 `reconcile::effective_priority`），不让系统自动节能限速。
    pub const GAME: Self = Self {
        priority: Some(PriorityClass::AboveNormal),
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    /// 正在玩、正在发声、用户要求保持快的程序：不提优先级、不限核心，只是不让系统自动节能限速。
    pub const PROTECTED: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::All,
    };

    /// 低优先级程序到了前台：仍在小核，但不节能，界面照样能点。
    pub const LOW_IN_FOREGROUND: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Off,
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
    /// 刚失去前台不久（保持期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
    /// 进程树正在发声，或刚停声不久（由调度器跨周期判断，切歌、暂停的间隙不来回切换）。
    pub audible: bool,
    /// 在后台持续占用处理器（由调度器跨周期判断）。
    pub busy: bool,
}

/// 正在被用户使用、绝不让路的程序。安全守卫（`safety::SafetyGuard::check_demote`）
/// 按同样的规则再把关一次：改这里时那边一起改。
pub fn in_use(v: &ProcessView) -> bool {
    let c = v.class;
    c.in_foreground_tree
        || v.recently_foreground
        || v.active_game
        || v.boosted
        || v.audible
        || matches!(
            c.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

/// 需要保护、不让系统自动节能限速的程序。连用户指定的"低优先级"也让步。
fn protected(v: &ProcessView) -> bool {
    v.active_game
        || v.boosted
        || v.audible
        || matches!(
            v.class.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

pub fn decide(v: &ProcessView) -> Desired {
    let c = v.class;
    if !c.is_schedulable() {
        return Desired::NEUTRAL;
    }
    if c.preference == AppPreference::Game {
        return Desired::GAME;
    }
    if protected(v) {
        return Desired::PROTECTED;
    }
    if c.preference == AppPreference::Background {
        return if c.in_foreground_tree {
            Desired::LOW_IN_FOREGROUND
        } else {
            Desired::YIELDED
        };
    }
    if v.busy && !in_use(v) {
        Desired::YIELDED
    } else {
        Desired::NEUTRAL
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

    fn view(c: &Classification, busy: bool) -> ProcessView<'_> {
        ProcessView {
            class: c,
            active_game: false,
            recently_foreground: false,
            boosted: false,
            audible: c.plays_audio,
            busy,
        }
    }

    #[test]
    fn only_marked_games_are_raised_and_only_one_level() {
        use AppPreference::*;
        assert_eq!(decide(&view(&class(Game, false), false)), Desired::GAME);
        assert_eq!(Desired::GAME.priority, Some(PriorityClass::AboveNormal));
        for pref in [Auto, AlwaysFast, Background] {
            for fg in [false, true] {
                for busy in [false, true] {
                    let c = class(pref, fg);
                    let d = decide(&view(&c, busy));
                    assert!(
                        d.priority
                            .is_none_or(|p| p.rank() < PriorityClass::Normal.rank()),
                        "{pref:?} 前台={fg} 忙={busy}：{d:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn only_busy_background_programs_yield() {
        let idle = class(AppPreference::Auto, false);
        assert_eq!(decide(&view(&idle, false)), Desired::NEUTRAL);
        assert_eq!(decide(&view(&idle, true)), Desired::YIELDED);
        let fg = class(AppPreference::Auto, true);
        assert_eq!(decide(&view(&fg, true)), Desired::NEUTRAL);
    }

    #[test]
    fn programs_in_use_never_yield() {
        let c = class(AppPreference::Auto, false);
        let recent = ProcessView {
            recently_foreground: true,
            ..view(&c, true)
        };
        let game = ProcessView {
            active_game: true,
            ..view(&c, true)
        };
        let boosted = ProcessView {
            boosted: true,
            ..view(&c, true)
        };
        assert_eq!(decide(&recent), Desired::NEUTRAL);
        for v in [game, boosted] {
            assert_eq!(decide(&v), Desired::PROTECTED);
        }
        let fast = class(AppPreference::AlwaysFast, false);
        assert_eq!(decide(&view(&fast, true)), Desired::PROTECTED);
        let game = class(AppPreference::Game, false);
        assert_eq!(decide(&view(&game, true)), Desired::GAME);
    }

    #[test]
    fn low_priority_programs_stay_low_unless_they_play_audio() {
        let bg = class(AppPreference::Background, false);
        assert_eq!(decide(&view(&bg, false)), Desired::YIELDED);
        let fg = class(AppPreference::Background, true);
        assert_eq!(decide(&view(&fg, false)), Desired::LOW_IN_FOREGROUND);
        let audio = Classification {
            plays_audio: true,
            ..bg
        };
        assert_eq!(decide(&view(&audio, true)), Desired::PROTECTED);
    }

    #[test]
    fn a_game_inheriting_low_priority_from_its_launcher_is_not_lowered() {
        let c = class(AppPreference::Background, false);
        let game = ProcessView {
            active_game: true,
            ..view(&c, false)
        };
        assert_eq!(decide(&game), Desired::PROTECTED);
    }

    #[test]
    fn audio_is_protected_from_system_throttling() {
        let c = Classification {
            plays_audio: true,
            ..class(AppPreference::Auto, false)
        };
        assert_eq!(decide(&view(&c, true)), Desired::PROTECTED);
    }

    #[test]
    fn excluded_processes_stay_neutral() {
        let c = Classification {
            role: Role::SystemOther,
            schedule: ScheduleScope::Excluded,
            ..class(AppPreference::Background, false)
        };
        assert_eq!(decide(&view(&c, true)), Desired::NEUTRAL);
    }
}
