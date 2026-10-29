//! 对每个程序（进程树）的判断：此刻是不是正在玩的游戏、刚用过、在发声、在忙……
//! 同一棵树里的进程按同一份判断决定（各自只多一项：自己是否在输出声音）。

use std::collections::{HashMap, HashSet};

use super::Scheduler;
use super::decisions::{Cause, Decision, ProgramState, Verdict};
use super::policy::{self, Desired, ProcessView};
use super::trackers::{Demand, Pressure};
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::model::{Pid, PriorityClass, Snapshot};
use crate::prefs::AppPreference;

impl Scheduler {
    /// 每个程序（进程树）此刻的情况。同一棵树里的进程一起判断，每轮每个程序只判断一次。
    pub(super) fn judge_programs(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        snap: &Snapshot,
    ) -> HashMap<Pid, Program> {
        let now = snap.at;
        let gaming = self.games.any_active();
        let elevated = self.self_elevated_roots(classes, snap);
        let demand = Demand {
            now,
            pressure: self.pressure,
            idle_for: self.idle_for,
        };
        let boosted = self.boosts.roots(classes);
        let roots: HashSet<Pid> = classes.values().map(|c| c.root_pid).collect();
        let mut programs = HashMap::with_capacity(roots.len());
        for root_pid in roots {
            let (Some(root_class), Some(root)) = (classes.get(&root_pid), self.load.root(root_pid))
            else {
                continue;
            };
            let active_game = self.game_roots.contains(&root_pid);
            let mut program = Program {
                active_game,
                gaming,
                needs_all_cores: active_game
                    && self.game_cores.needs_all_cores(
                        root,
                        self.load.of(&root).core_pct,
                        self.cfg.cores.performance_threads,
                    ),
                recently_foreground: self.foreground.recently_foreground(root_pid),
                boosted: boosted.contains(&root_pid),
                audible: root_class.plays_audio || self.audio.audible(root_pid),
                self_elevated: elevated.contains(&root_pid),
                busy: false,
                busy_with_audio: false,
            };
            let load = self.load.of(&root);
            program.busy = self.busy.observe(
                root,
                load,
                policy::in_use(&program.view(root_class, false)),
                demand,
                self.cfg.busy,
            );
            // 发声的程序：只在玩游戏时、除了发声之外没在用，才让到小核。
            let silent = Program {
                audible: false,
                ..program
            };
            let in_use_besides_audio =
                !program.audible || !gaming || policy::in_use(&silent.view(root_class, false));
            program.busy_with_audio = self.busy_with_audio.observe(
                root,
                load,
                in_use_besides_audio,
                demand,
                self.cfg.busy_with_audio,
            );
            programs.insert(root_pid, program);
        }
        programs
    }

    /// 每个程序按主进程算的决定，和上一轮比较。
    pub(super) fn record_decisions(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        programs: &HashMap<Pid, Program>,
    ) -> Vec<Decision> {
        let states: Vec<_> = programs
            .iter()
            .filter_map(|(root_pid, program)| {
                let root = self.load.root(*root_pid)?;
                let class = classes.get(root_pid)?;
                let verdict = Verdict::of(self.desired(class, program, false));
                let usage = self.load.of(&root);
                let cause = match verdict {
                    Verdict::Normal => Some(self.cause(class, program)),
                    Verdict::Yielded if class.preference == AppPreference::Background => {
                        Some(Cause::MarkedLow)
                    }
                    _ => None,
                };
                Some((
                    root,
                    ProgramState {
                        verdict,
                        load_pct: usage.core_pct.round() as u32,
                        disk_mb_s: (usage.disk_bytes_per_s / (1024.0 * 1024.0)).round() as u32,
                        cause,
                    },
                ))
            })
            .collect();
        self.decisions.update(self.pressure, states)
    }

    /// 主进程自己把优先级调到"高"或更高的程序（按 Poise 调整之前的优先级）。
    fn self_elevated_roots(
        &self,
        classes: &HashMap<Pid, Classification>,
        snap: &Snapshot,
    ) -> HashSet<Pid> {
        snap.processes
            .iter()
            .filter(|p| {
                let key = p.facts.key;
                classes.get(&key.pid).is_some_and(|c| c.root_pid == key.pid)
                    && self
                        .reconciler
                        .original_priority(&key)
                        .or(p.facts.priority)
                        .is_some_and(|o| o.rank() >= PriorityClass::High.rank())
            })
            .map(|p| p.facts.key.pid)
            .collect()
    }

    /// 程序没有让路的原因。
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

    pub(super) fn desired(
        &self,
        class: &Classification,
        program: &Program,
        emitting: bool,
    ) -> Desired {
        let mut desired = policy::decide(&program.view(class, emitting));
        if !self.cfg.cores.allows(desired.cores) {
            desired.cores = CoreSet::All;
        }
        desired
    }
}

/// 一个程序（进程树）此刻的情况，由调度器跨周期判断；树里的每个进程都按它决定。
#[derive(Debug, Clone, Copy)]
pub(super) struct Program {
    active_game: bool,
    gaming: bool,
    needs_all_cores: bool,
    recently_foreground: bool,
    boosted: bool,
    audible: bool,
    /// 主进程自己把优先级调到了"高"或更高。
    self_elevated: bool,
    busy: bool,
    busy_with_audio: bool,
}

impl Program {
    /// 树里某个进程的情况：程序的情况加上它自己是否在输出声音。
    pub(super) fn view<'a>(&self, class: &'a Classification, emitting: bool) -> ProcessView<'a> {
        ProcessView {
            class,
            active_game: self.active_game,
            gaming: self.gaming,
            needs_all_cores: self.needs_all_cores,
            recently_foreground: self.recently_foreground,
            boosted: self.boosted,
            audible: self.audible,
            emitting,
            self_elevated: self.self_elevated,
            busy: self.busy,
            busy_with_audio: self.busy_with_audio,
        }
    }
}
