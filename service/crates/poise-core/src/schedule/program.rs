//! 对每个程序（进程树）的判断：此刻是不是正在玩的游戏、刚用过、在发声、在忙……
//! 同一棵树里的进程按同一份判断决定（各自只多一项：自己是否在输出声音）。

use std::collections::{HashMap, HashSet};

use super::Scheduler;
use super::decisions::{Decision, Verdict};
use super::policy::{self, Desired, ProcessView};
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::model::Pid;

impl Scheduler {
    /// 每个程序（进程树）此刻的情况。同一棵树里的进程一起判断，每轮每个程序只判断一次。
    pub(super) fn judge_programs(
        &mut self,
        classes: &HashMap<Pid, Classification>,
    ) -> HashMap<Pid, Program> {
        let gaming = self.games.any_active();
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
                        self.load.of(&root),
                        self.cfg.cores.performance_threads,
                    ),
                recently_foreground: self.foreground.recently_foreground(root_pid),
                boosted: boosted.contains(&root_pid),
                audible: root_class.plays_audio || self.audio.audible(root_pid),
                busy: false,
                busy_with_audio: false,
            };
            let load = self.load.of(&root);
            program.busy = self.busy.observe(
                root,
                load,
                policy::in_use(&program.view(root_class, false)),
                self.pressure,
                self.idle_for,
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
                self.pressure,
                self.idle_for,
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
        let verdicts: Vec<_> = programs
            .iter()
            .filter_map(|(root_pid, program)| {
                let root = self.load.root(*root_pid)?;
                let class = classes.get(root_pid)?;
                let verdict = Verdict::of(self.desired(class, program, false));
                Some((root, verdict, self.load.of(&root).round() as u32))
            })
            .collect();
        self.decisions.update(self.pressure, verdicts)
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
            busy: self.busy,
            busy_with_audio: self.busy_with_audio,
        }
    }
}
