//! 调度策略：根据一个进程的情况，决定它"应该"处于什么状态。纯函数，不记状态。
//!
//! 原则是**让路为主，基本不提速**：大幅提速会和声音、输入、系统组件抢资源，反而卡顿。Poise 主要把在后台
//! 持续占用处理器的程序挪开，让正在用的程序自然拿到资源。唯一的提速是用户标记为"游戏"的程序：
//! 提高一级（正常 → 高于正常），不会更高。
//!
//! 另外弥补 Windows 的短板：
//! - Windows 会自动给后台、最小化的程序降频、放宽计时器精度，游戏切出去、后台音乐和语音会因此卡顿。
//!   对这些程序明确关闭系统的自动节能。
//! - Windows 有时把重点程序的线程放到慢的核心上（只靠一两个主线程的游戏，主线程落到小核帧数就掉），
//!   又让别的程序的零碎工作打断重点程序。重点程序（正在玩的游戏，以及正被直接使用、持续需要处理器的
//!   任何程序，见 `trackers::focus`）按需要的线程占用主域（大核），不够再逐个加上副域；其余没在用的
//!   程序让到副域，同一个程序固定在同一个域里（见 `placement`）。核心域见 `CoreSet`：大小核处理器上
//!   主域是大核、副域是按缓存分组的小核；多 CCD 的处理器上是各个 CCD；只有一个域的处理器不限制核心。
//!
//! 下表的"大核""小核"指主域、副域。
//!
//! | 情况 | 状态 |
//! |---|---|
//! | 用户标记为"游戏" | 大核 · 优先级高一级 · 不受系统节能限速 |
//! | 正在玩的游戏（自动识别） | 大核 · 不受系统节能限速 |
//! | 正被直接使用、持续需要处理器的程序（前台的编译、渲染等） | 大核，其余照常 |
//! | 上面几种重点程序，需要的线程超过大核能提供的 | 逐个加上副域，直到不限核心 |
//! | 始终优先、临时全速 | 不受系统节能限速 |
//! | 正在发声的程序（整个进程树，停声后保持一会儿）里输出声音的进程 | 不受系统节能限速 |
//! | 正在发声的程序里的其他进程 | 不调整（不让路，也不替程序做主：浏览器会自己给后台标签页节能） |
//! | 正在发声、有重点程序时又在后台忙 | 小核 · 不受系统节能限速（声音照样流畅，大核留给重点程序） |
//! | 前台、刚用过的程序 | 不调整（Windows 本来就照顾前台）；玩游戏时"刚用过"不再豁免，只有前台算 |
//! | 自己把优先级调到"高"的程序（录屏、串流、音频工作站开工时会这样做）在后台忙 | 不调整；有重点程序时只让到小核，同发声的程序 |
//! | 低优先级（用户指定） | 小核 · 低于正常 · 在后台节能 |
//! | 自动，在后台持续占用处理器，有重点程序、发热或用电池时 | 小核 · 低于正常 · 节能（保持到它回到前台、发声或退出） |
//! | 同上，但负载轻（达不到需求一般时的门槛：手柄映射、录像、副屏视频） | 小核 · 低于正常（不节能：节能会让输入变迟、画面掉帧） |
//! | 自动，在后台持续占用处理器，只是正在用的程序在忙 | 低于正常（核心不限、不节能：空闲的核心照样给它用） |
//! | 自动，让路之后没人需要处理器了、它还在忙 | 低于正常（同上；闲下来才恢复，不来回调整） |
//! | 自动，在后台闲着，有重点程序时 | 小核（不降优先级、不节能；大核留给重点程序） |
//! | 自动，在后台闲着 | 不调整（闲着的程序不和谁抢资源，不值得动它） |
//!
//! 系统进程不在调度范围内。

use super::decisions::Verdict;
use super::desired::Desired;
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::prefs::AppPreference;

/// 单个进程的情况。
#[derive(Debug, Clone, Copy)]
pub struct ProcessView<'a> {
    pub class: &'a Classification,
    /// 属于正在玩的游戏（游戏本身及其进程树，包括切到后台的游戏）。
    pub active_game: bool,
    /// 有游戏正在运行：此时只有前台算正在用，刚用过的程序（例如启动游戏的客户端）不再豁免。
    pub gaming: bool,
    /// 它是重点程序时，按它需要的线程算出的核心范围（见 `trackers::Footprint`）。
    pub focus: Option<CoreSet>,
    /// 有重点程序（或刚结束不久）：没在用的程序让到小核。
    pub focusing: bool,
    /// 刚失去前台不久（保持期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
    /// 进程树正在发声，或刚停声不久（由调度器跨周期判断，切歌、暂停的间隙不来回切换）。
    pub audible: bool,
    /// 这个进程本身在输出声音，或刚停下不久。
    pub emitting: bool,
    /// 程序的主进程自己把优先级调到了"高"或更高：它在声明自己的工作对时间敏感。
    pub self_elevated: bool,
    /// 在后台持续占用处理器（由调度器跨周期判断）。
    pub busy: bool,
    /// 在忙，但负载轻（见 `BusyTracker::heavy`）：完整让路时不节能。
    pub light: bool,
    /// 正在发声，有重点程序时又在后台忙（由调度器跨周期判断）：只让到小核。
    pub busy_with_audio: bool,
    /// 刚停止使用不久：先不让到小核，来回切换窗口、通知弹窗抢一下前台不会让它来回挪动。
    pub settling: bool,
    /// 在忙的程序完整让路（需求高、用电池时），否则只降优先级。
    pub yield_fully: bool,
}

/// 正在被用户使用、绝不让路的程序：用户直接在用的（见 `Classification::in_use_by_user`，安全守卫
/// 按它再把关一次），加上跨周期才看得出来的——刚用过、正在玩、临时全速、在发声。
pub fn in_use(v: &ProcessView) -> bool {
    v.class.in_use_by_user()
        || (v.recently_foreground && !v.gaming)
        || v.active_game
        || v.boosted
        || v.audible
}

/// 用户要求保持快的程序：不让系统自动节能限速。
fn kept_fast(v: &ProcessView) -> bool {
    v.boosted
        || matches!(
            v.class.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

/// 进程应有的状态，以及按什么调度（供日志）。
pub fn decide(v: &ProcessView) -> (Desired, Verdict) {
    let (mut desired, mut verdict) = decide_cores_freely(v);
    // 重点程序：在大核上（按需要逐级放开）。用户设为"低优先级"的程序除外，照用户的留在小核。
    if let Some(cores) = v.focus
        && v.class.is_schedulable()
        && matches!(desired.cores, CoreSet::All | CoreSet::Performance)
    {
        desired.cores = cores;
        if verdict == Verdict::Normal {
            verdict = Verdict::Focused;
        }
    }
    (desired, verdict)
}

fn decide_cores_freely(v: &ProcessView) -> (Desired, Verdict) {
    use Verdict::*;
    let c = v.class;
    if c.is_maintenance() {
        return if v.focusing && !in_use(v) {
            (Desired::MAINTENANCE_ASIDE, Aside)
        } else {
            (Desired::NEUTRAL, Normal)
        };
    }
    if !c.is_schedulable() {
        return (Desired::NEUTRAL, Normal);
    }
    if c.preference == AppPreference::Game {
        return (Desired::GAME, Playing);
    }
    if v.active_game {
        return (Desired::PLAYING, Playing);
    }
    if v.busy_with_audio {
        return (Desired::BACKSEAT, Backseat);
    }
    if kept_fast(v) {
        return (Desired::PROTECTED, Normal);
    }
    // 发声的程序连用户指定的"低优先级"也让步：输出声音的进程不受限速，其余进程不动。
    if v.audible {
        let desired = if v.emitting {
            Desired::PROTECTED
        } else {
            Desired::NEUTRAL
        };
        return (desired, Normal);
    }
    if c.preference == AppPreference::Background {
        return if c.in_foreground_tree {
            (Desired::LOW_IN_FOREGROUND, Yielded)
        } else {
            (Desired::YIELDED, Yielded)
        };
    }
    if v.busy && v.self_elevated && !in_use(v) {
        // 降优先级、节能限速都会让录像、串流掉帧；有重点程序时让到小核就够了，大核留给重点程序。
        return if v.focusing {
            (Desired::BACKSEAT, Backseat)
        } else {
            (Desired::NEUTRAL, Normal)
        };
    }
    if v.busy && !in_use(v) {
        if v.yield_fully && v.light {
            (Desired::NUDGED, Yielded)
        } else if v.yield_fully {
            (Desired::YIELDED, Yielded)
        } else {
            (Desired::LOWERED, Lowered)
        }
    } else if v.focusing && v.focus.is_none() && !v.settling && !in_use(v) {
        (Desired::ASIDE, Aside)
    } else {
        (Desired::NEUTRAL, Normal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Role, ScheduleScope};
    use crate::model::PriorityClass;

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

    fn decide_state(v: &ProcessView) -> Desired {
        decide(v).0
    }

    fn view(c: &Classification, busy: bool) -> ProcessView<'_> {
        ProcessView {
            class: c,
            active_game: false,
            gaming: false,
            focus: None,
            focusing: false,
            recently_foreground: false,
            boosted: false,
            audible: c.plays_audio,
            emitting: c.emits_audio,
            self_elevated: false,
            busy,
            light: false,
            busy_with_audio: false,
            settling: false,
            yield_fully: true,
        }
    }

    #[test]
    fn light_busy_programs_move_aside_without_being_throttled() {
        let c = class(AppPreference::Auto, false);
        let light = ProcessView {
            light: true,
            ..view(&c, true)
        };
        assert_eq!(decide(&light), (Desired::NUDGED, Verdict::Yielded));
        assert_eq!(Desired::NUDGED.eco, crate::action::EcoQos::Default);
        // 只降优先级的时候不区分轻重。
        let lowered = ProcessView {
            yield_fully: false,
            ..light
        };
        assert_eq!(decide_state(&lowered), Desired::LOWERED);
    }

    #[test]
    fn only_marked_games_are_raised_and_only_one_level() {
        use AppPreference::*;
        assert_eq!(
            decide_state(&view(&class(Game, false), false)),
            Desired::GAME
        );
        assert_eq!(Desired::GAME.priority, Some(PriorityClass::AboveNormal));
        assert_eq!(Desired::PLAYING.priority, None);
        for pref in [Auto, AlwaysFast, Background] {
            for fg in [false, true] {
                for busy in [false, true] {
                    let c = class(pref, fg);
                    let d = decide_state(&view(&c, busy));
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
        assert_eq!(decide_state(&view(&idle, false)), Desired::NEUTRAL);
        assert_eq!(decide_state(&view(&idle, true)), Desired::YIELDED);
        let fg = class(AppPreference::Auto, true);
        assert_eq!(decide_state(&view(&fg, true)), Desired::NEUTRAL);
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
        assert_eq!(decide_state(&recent), Desired::NEUTRAL);
        assert_eq!(decide_state(&game), Desired::PLAYING);
        assert_eq!(decide_state(&boosted), Desired::PROTECTED);
        let fast = class(AppPreference::AlwaysFast, false);
        assert_eq!(decide_state(&view(&fast, true)), Desired::PROTECTED);
        let game = class(AppPreference::Game, false);
        assert_eq!(decide_state(&view(&game, true)), Desired::GAME);
    }

    #[test]
    fn low_priority_programs_stay_low_unless_they_play_audio() {
        let bg = class(AppPreference::Background, false);
        assert_eq!(decide_state(&view(&bg, false)), Desired::YIELDED);
        let fg = class(AppPreference::Background, true);
        assert_eq!(decide_state(&view(&fg, false)), Desired::LOW_IN_FOREGROUND);
        let audio = Classification {
            plays_audio: true,
            emits_audio: true,
            ..bg
        };
        assert_eq!(decide_state(&view(&audio, true)), Desired::PROTECTED);
    }

    #[test]
    fn programs_that_raised_their_own_priority_are_not_lowered() {
        let c = class(AppPreference::Auto, false);
        let elevated = ProcessView {
            self_elevated: true,
            ..view(&c, true)
        };
        assert_eq!(decide_state(&elevated), Desired::NEUTRAL);
        let focusing = ProcessView {
            focusing: true,
            ..elevated
        };
        assert_eq!(decide_state(&focusing), Desired::BACKSEAT);
        // 用户自己设为"低优先级"的，照用户的。
        let low = class(AppPreference::Background, false);
        assert_eq!(
            decide_state(&ProcessView {
                self_elevated: true,
                ..view(&low, true)
            }),
            Desired::YIELDED
        );
    }

    #[test]
    fn a_game_inheriting_low_priority_from_its_launcher_is_not_lowered() {
        let c = class(AppPreference::Background, false);
        let game = ProcessView {
            active_game: true,
            ..view(&c, false)
        };
        assert_eq!(decide_state(&game), Desired::PLAYING);
    }

    #[test]
    fn audio_is_protected_from_system_throttling() {
        let c = Classification {
            plays_audio: true,
            emits_audio: true,
            ..class(AppPreference::Auto, false)
        };
        assert_eq!(decide_state(&view(&c, true)), Desired::PROTECTED);
    }

    #[test]
    fn windows_maintenance_only_steps_aside_while_something_is_in_focus() {
        let mut c = class(AppPreference::Auto, false);
        c.schedule = ScheduleScope::Maintenance;
        let mut v = view(&c, true);
        assert_eq!(decide(&v), (Desired::NEUTRAL, Verdict::Normal));
        v.focusing = true;
        assert_eq!(decide(&v), (Desired::MAINTENANCE_ASIDE, Verdict::Aside));
    }

    #[test]
    fn excluded_processes_stay_neutral() {
        let c = Classification {
            role: Role::SystemOther,
            schedule: ScheduleScope::Excluded,
            ..class(AppPreference::Background, false)
        };
        assert_eq!(decide_state(&view(&c, true)), Desired::NEUTRAL);
    }

    #[test]
    fn while_gaming_only_the_foreground_counts_as_in_use() {
        let c = class(AppPreference::Auto, false);
        let recent = ProcessView {
            recently_foreground: true,
            ..view(&c, true)
        };
        assert_eq!(decide_state(&recent), Desired::NEUTRAL);
        let while_gaming = ProcessView {
            gaming: true,
            ..recent
        };
        assert_eq!(decide_state(&while_gaming), Desired::YIELDED);
        let fg = class(AppPreference::Auto, true);
        let fg_while_gaming = ProcessView {
            gaming: true,
            ..view(&fg, true)
        };
        assert_eq!(decide_state(&fg_while_gaming), Desired::NEUTRAL);
    }

    #[test]
    fn focused_programs_get_the_performance_cores_and_idle_ones_step_aside() {
        let c = class(AppPreference::Auto, true);
        let focused = ProcessView {
            focus: Some(CoreSet::Performance),
            focusing: true,
            ..view(&c, false)
        };
        assert_eq!(
            decide(&focused),
            (
                Desired {
                    cores: CoreSet::Performance,
                    ..Desired::NEUTRAL
                },
                Verdict::Focused
            )
        );
        // 游戏需要更多线程：多用一组小核。
        let game = ProcessView {
            active_game: true,
            focus: Some(CoreSet::Domains(0b11)),
            ..focused
        };
        assert_eq!(decide(&game).0.cores, CoreSet::Domains(0b11));
        // 其余没在用的程序让到小核；正在用的不动。
        let idle = class(AppPreference::Auto, false);
        let aside = ProcessView {
            focusing: true,
            ..view(&idle, false)
        };
        assert_eq!(decide(&aside), (Desired::ASIDE, Verdict::Aside));
        let audible = ProcessView {
            audible: true,
            ..aside
        };
        assert_eq!(decide(&audible), (Desired::NEUTRAL, Verdict::Normal));
        // 用户设为"低优先级"的程序到了前台、又很忙：照用户的，留在小核。
        let low = class(AppPreference::Background, true);
        let low_focus = ProcessView {
            focus: Some(CoreSet::Performance),
            ..view(&low, true)
        };
        assert_eq!(decide(&low_focus).0, Desired::LOW_IN_FOREGROUND);
    }
}
