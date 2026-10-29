//! 调度策略：根据一个进程的情况，决定它"应该"处于什么状态。纯函数，不记状态。
//!
//! 原则是**让路为主，基本不提速**：大幅提速会和声音、输入、系统组件抢资源，反而卡顿。Poise 主要把在后台
//! 持续占用处理器的程序挪开，让正在用的程序自然拿到资源。唯一的提速是用户标记为"游戏"的程序：
//! 提高一级（正常 → 高于正常），不会更高。
//!
//! 另外弥补 Windows 的短板：
//! - Windows 会自动给后台、最小化的程序降频、放宽计时器精度，游戏切出去、后台音乐和语音会因此卡顿。
//!   对这些程序明确关闭系统的自动节能。
//! - 大小核处理器上，Windows 有时把游戏的线程放到小核（尤其是只靠一两个主线程的游戏，主线程落到小核
//!   帧数就掉）。游戏固定在大核上运行；后台忙碌的程序让到小核，大核留给游戏。
//!
//! | 情况 | 状态 |
//! |---|---|
//! | 用户标记为"游戏" | 大核 · 优先级高一级 · 不受系统节能限速 |
//! | 正在玩的游戏（自动识别） | 大核 · 不受系统节能限速 |
//! | 上面两种，但需要的线程超过大核能提供的（重度多线程游戏） | 不限核心，其余同上 |
//! | 始终优先、临时全速 | 不受系统节能限速 |
//! | 正在发声的程序（整个进程树，停声后保持一会儿）里输出声音的进程 | 不受系统节能限速 |
//! | 正在发声的程序里的其他进程 | 不调整（不让路，也不替程序做主：浏览器会自己给后台标签页节能） |
//! | 正在发声、玩游戏时又在后台忙 | 小核 · 不受系统节能限速（声音照样流畅，大核留给游戏） |
//! | 前台、刚用过的程序 | 不调整（Windows 本来就照顾前台）；玩游戏时"刚用过"不再豁免，只有前台算 |
//! | 低优先级（用户指定） | 小核 · 低于正常 · 在后台节能 |
//! | 自动，在后台持续占用处理器 | 小核 · 低于正常 · 节能（保持到它回到前台、发声或退出） |
//! | 自动，在后台闲着 | 不调整（闲着的程序不和谁抢资源，不值得动它） |
//!
//! 系统进程不在调度范围内。

use crate::action::{CoreSet, EcoQos, ResourcePriority};
use crate::classify::Classification;
use crate::model::PriorityClass;
use crate::prefs::AppPreference;

/// 一个进程期望的调度状态。`priority` 为 None 表示保持原始优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Desired {
    pub priority: Option<PriorityClass>,
    pub eco: EcoQos,
    pub cores: CoreSet,
    /// 内存与磁盘读写的优先级。
    pub resources: ResourcePriority,
}

impl Desired {
    pub const NEUTRAL: Self = Self {
        priority: None,
        eco: EcoQos::Default,
        cores: CoreSet::All,
        resources: ResourcePriority::Default,
    };

    /// 在后台让出性能：小核、低于正常、节能；内存紧张时先回收它的内存，保护正在用的程序。
    pub const YIELDED: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::On,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Low,
    };

    /// 用户标记的游戏：在大核上运行，优先级提高一级（高于正常，见 `reconcile::effective_priority`），
    /// 不让系统自动节能限速。
    pub const GAME: Self = Self {
        priority: Some(PriorityClass::AboveNormal),
        eco: EcoQos::Off,
        cores: CoreSet::Performance,
        resources: ResourcePriority::Default,
    };

    /// 自动识别的、正在玩的游戏：在大核上运行，不让系统自动节能限速。
    pub const PLAYING: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::Performance,
        resources: ResourcePriority::Default,
    };

    /// 正在发声、用户要求保持快的程序：不提优先级、不限核心，只是不让系统自动节能限速。
    pub const PROTECTED: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::All,
        resources: ResourcePriority::Default,
    };

    /// 正在发声、玩游戏时又在后台忙的程序：只让到小核。不降优先级、不节能——声音线程有自己的
    /// 实时优先级，在小核上照样流畅；大核留给游戏。
    pub const BACKSEAT: Self = Self {
        priority: None,
        eco: EcoQos::Off,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    /// 低优先级程序到了前台：仍在小核，但不节能，界面照样能点。
    pub const LOW_IN_FOREGROUND: Self = Self {
        priority: Some(PriorityClass::BelowNormal),
        eco: EcoQos::Off,
        cores: CoreSet::Efficiency,
        resources: ResourcePriority::Default,
    };

    pub fn is_neutral(&self) -> bool {
        *self == Self::NEUTRAL
    }
}

/// 单个进程的情况。
#[derive(Debug, Clone, Copy)]
pub struct ProcessView<'a> {
    pub class: &'a Classification,
    /// 属于正在玩的游戏（游戏本身及其进程树，包括切到后台的游戏）。
    pub active_game: bool,
    /// 有游戏正在运行：此时只有前台算正在用，刚用过的程序（例如启动游戏的客户端）不再豁免。
    pub gaming: bool,
    /// 这个游戏需要的线程超过大核能提供的：不固定在大核。
    pub needs_all_cores: bool,
    /// 刚失去前台不久（保持期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
    /// 进程树正在发声，或刚停声不久（由调度器跨周期判断，切歌、暂停的间隙不来回切换）。
    pub audible: bool,
    /// 这个进程本身在输出声音，或刚停下不久。
    pub emitting: bool,
    /// 在后台持续占用处理器（由调度器跨周期判断）。
    pub busy: bool,
    /// 正在发声，玩游戏时又在后台忙（由调度器跨周期判断）：只让到小核。
    pub busy_with_audio: bool,
}

/// 正在被用户使用、绝不让路的程序。安全守卫（`safety::SafetyGuard::check_demote`）
/// 按同样的规则再把关一次：改这里时那边一起改。
pub fn in_use(v: &ProcessView) -> bool {
    let c = v.class;
    c.in_foreground_tree
        || (v.recently_foreground && !v.gaming)
        || v.active_game
        || v.boosted
        || v.audible
        || matches!(
            c.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

/// 用户要求保持快的程序：不让系统自动节能限速。
fn kept_fast(v: &ProcessView) -> bool {
    v.boosted
        || matches!(
            v.class.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

pub fn decide(v: &ProcessView) -> Desired {
    let mut desired = decide_cores_freely(v);
    if v.needs_all_cores && desired.cores == CoreSet::Performance {
        desired.cores = CoreSet::All;
    }
    desired
}

fn decide_cores_freely(v: &ProcessView) -> Desired {
    let c = v.class;
    if !c.is_schedulable() {
        return Desired::NEUTRAL;
    }
    if c.preference == AppPreference::Game {
        return Desired::GAME;
    }
    if v.active_game {
        return Desired::PLAYING;
    }
    if v.busy_with_audio {
        return Desired::BACKSEAT;
    }
    if kept_fast(v) {
        return Desired::PROTECTED;
    }
    // 发声的程序连用户指定的"低优先级"也让步：输出声音的进程不受限速，其余进程不动。
    if v.audible {
        return if v.emitting {
            Desired::PROTECTED
        } else {
            Desired::NEUTRAL
        };
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
            emits_audio: false,
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
            gaming: false,
            needs_all_cores: false,
            recently_foreground: false,
            boosted: false,
            audible: c.plays_audio,
            emitting: c.emits_audio,
            busy,
            busy_with_audio: false,
        }
    }

    #[test]
    fn only_marked_games_are_raised_and_only_one_level() {
        use AppPreference::*;
        assert_eq!(decide(&view(&class(Game, false), false)), Desired::GAME);
        assert_eq!(Desired::GAME.priority, Some(PriorityClass::AboveNormal));
        assert_eq!(Desired::PLAYING.priority, None);
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
        assert_eq!(decide(&game), Desired::PLAYING);
        assert_eq!(decide(&boosted), Desired::PROTECTED);
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
            emits_audio: true,
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
        assert_eq!(decide(&game), Desired::PLAYING);
    }

    #[test]
    fn audio_is_protected_from_system_throttling() {
        let c = Classification {
            plays_audio: true,
            emits_audio: true,
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

    #[test]
    fn while_gaming_only_the_foreground_counts_as_in_use() {
        let c = class(AppPreference::Auto, false);
        let recent = ProcessView {
            recently_foreground: true,
            ..view(&c, true)
        };
        assert_eq!(decide(&recent), Desired::NEUTRAL);
        let while_gaming = ProcessView {
            gaming: true,
            ..recent
        };
        assert_eq!(decide(&while_gaming), Desired::YIELDED);
        let fg = class(AppPreference::Auto, true);
        let fg_while_gaming = ProcessView {
            gaming: true,
            ..view(&fg, true)
        };
        assert_eq!(decide(&fg_while_gaming), Desired::NEUTRAL);
    }
}
