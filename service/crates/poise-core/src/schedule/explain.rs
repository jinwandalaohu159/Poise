//! 解释决定：每个程序按主进程算的决定、负载、核心范围，以及没有让路时的原因，交给 [`super::decisions`]
//! 和上一轮比较、只报告变化；这一轮的全部结果留着，供界面查看（核心与缓存面板）。

use super::Scheduler;
use super::decisions::{Cause, Decision, ProgramState, Verdict};
use super::program::Program;
use super::trackers::{FocusEnd, Pressure};
use crate::classify::Classification;
use crate::fast_hash::HashMap;
use crate::model::{Millis, Pid};
use crate::prefs::AppPreference;

impl Scheduler {
    /// 每个程序按主进程算的决定，和上一轮比较。
    pub(super) fn record_decisions(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        programs: &HashMap<Pid, Program>,
        now: Millis,
    ) -> Vec<Decision> {
        let states: Vec<_> = programs
            .iter()
            .filter_map(|(root_pid, program)| {
                let root = self.load.root(*root_pid)?;
                let class = classes.get(&program.class_pid)?;
                let (desired, verdict) = self.desired(class, program, false);
                let usage = self.load.of(&root);
                let cause = match verdict {
                    Verdict::Normal => Some(
                        program
                            .focus_end
                            .map_or_else(|| self.cause(class, program), focus_end_cause),
                    ),
                    Verdict::Focused => Some(focus_cause(class, program)),
                    Verdict::Yielded if class.preference == AppPreference::Background => {
                        Some(Cause::MarkedLow)
                    }
                    _ => None,
                };
                Some((
                    root,
                    ProgramState {
                        verdict,
                        cores: desired.cores,
                        load_pct: usage.core_pct.round() as u32,
                        disk_mb_s: (usage.disk_bytes_per_s / (1024.0 * 1024.0)).round() as u32,
                        cause,
                    },
                ))
            })
            .collect();
        let decisions = self.decisions.update(
            self.pressure,
            self.demand_reason,
            states.iter().copied(),
            now,
        );
        self.explained = states;
        decisions
    }

    /// 程序没有让路（或者恢复）的原因。
    fn cause(&self, class: &Classification, program: &Program) -> Cause {
        if class.in_foreground_tree {
            Cause::Foreground
        } else if program.audible {
            Cause::Audio
        } else if program.boosted {
            Cause::Boosted
        } else if matches!(
            class.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        ) {
            Cause::Marked
        } else if program.recently_foreground && !program.gaming {
            Cause::RecentlyUsed
        } else if program.self_elevated && program.busy {
            Cause::SelfElevated
        } else if self.pressure == Pressure::None {
            Cause::NoDemand
        } else {
            Cause::LessDemand
        }
    }
}

/// 为什么是重点程序。
fn focus_cause(class: &Classification, program: &Program) -> Cause {
    if program.pending_game {
        Cause::LooksLikeGame
    } else if program.boosted {
        Cause::Boosted
    } else if matches!(
        class.preference,
        AppPreference::AlwaysFast | AppPreference::Game
    ) {
        Cause::Marked
    } else if class.in_foreground_tree {
        Cause::Foreground
    } else {
        Cause::RecentlyUsed
    }
}

/// 重点程序为什么结束了。
fn focus_end_cause(end: FocusEnd) -> Cause {
    match end {
        FocusEnd::LoadDropped => Cause::LoadDropped,
        FocusEnd::LeftForeground => Cause::LeftForeground,
    }
}
