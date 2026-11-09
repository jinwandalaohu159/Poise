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
        working: false,
        work_room: WorkRoom::Free,
        on_battery: false,
    }
}

#[test]
fn work_left_running_in_the_background_is_not_treated_as_background() {
    use crate::action::EcoQos;
    let c = class(AppPreference::Auto, false);
    let work = ProcessView {
        working: true,
        ..view(&c, true)
    };
    assert_eq!(decide(&work), (Desired::WORK, Verdict::Working));
    assert_eq!(Desired::WORK.eco, EcoQos::Off);
    // 前台也在干重活：优选的 4 个大核留给它，这份工作用其余核心、不节能。
    let shared = ProcessView {
        focusing: true,
        work_room: WorkRoom::Shared(CoreSet::Rest(4)),
        ..work
    };
    assert_eq!(
        decide_state(&shared),
        Desired {
            cores: CoreSet::Rest(4),
            ..Desired::WORK_SHARED
        }
    );
    assert_eq!(Desired::WORK_SHARED.eco, EcoQos::Off);
    // 重点程序要整个大核：移到小核。
    let squeezed = ProcessView {
        work_room: WorkRoom::Aside,
        ..shared
    };
    assert_eq!(decide_state(&squeezed), Desired::WORK_ASIDE);
    // 在玩游戏：移到小核并节能，功耗留给游戏；在发声、自己提了优先级的只让核心。
    let gaming = ProcessView {
        gaming: true,
        work_room: WorkRoom::Yielded(CoreSet::Efficiency),
        ..shared
    };
    assert_eq!(decide(&gaming), (Desired::WORK_YIELDED, Verdict::Working));
    assert_eq!(Desired::WORK_YIELDED.eco, EcoQos::On);
    let streaming = ProcessView {
        self_elevated: true,
        ..gaming
    };
    assert_eq!(
        decide_state(&streaming),
        Desired {
            priority: None,
            eco: EcoQos::Off,
            ..Desired::WORK_YIELDED
        }
    );
    let battery = ProcessView {
        on_battery: true,
        ..work
    };
    assert_eq!(decide_state(&battery), Desired::LOWERED);
    // 用户设为"低优先级"的不会被判断为在干活（见 `judge`）；判断了也照用户的。
    let low = class(AppPreference::Background, false);
    assert_eq!(
        decide_state(&ProcessView {
            working: false,
            ..view(&low, true)
        }),
        Desired::YIELDED
    );
}

#[test]
fn work_in_use_is_never_slowed_and_shares_only_when_someone_else_needs_it() {
    let fg = class(AppPreference::Auto, true);
    // 正在用、又在干活：全用、不节能；成了重点程序时按重点程序报告，但照样不节能。
    let used = ProcessView {
        working: true,
        ..view(&fg, true)
    };
    assert_eq!(decide(&used), (Desired::WORK, Verdict::Normal));
    let focused = ProcessView {
        focus: Some(CoreSet::Performance),
        ..used
    };
    assert_eq!(
        decide(&focused),
        (
            Desired {
                cores: CoreSet::Performance,
                ..Desired::WORK
            },
            Verdict::Focused
        )
    );
    // 在后台干活，正在用的别的程序在忙：优先级让一步，核心照用。
    let bg = class(AppPreference::Auto, false);
    let contended = ProcessView {
        working: true,
        work_room: WorkRoom::Contended,
        ..view(&bg, true)
    };
    assert_eq!(decide_state(&contended), Desired::WORK_SHARED);
    // 用电池：正在用的不调整，后台的只降优先级。
    assert_eq!(
        decide_state(&ProcessView {
            on_battery: true,
            ..used
        }),
        Desired::NEUTRAL
    );
    assert_eq!(
        decide_state(&ProcessView {
            on_battery: true,
            ..contended
        }),
        Desired::LOWERED
    );
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
    // 自动识别的游戏不提优先级，只保证不低于正常（见 `Desired::PLAYING`）。
    assert_eq!(Desired::PLAYING.priority, Some(PriorityClass::Normal));
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
