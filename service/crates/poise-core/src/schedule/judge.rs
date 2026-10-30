//! 对每个程序（进程树）的判断：此刻是不是正在玩的游戏、刚用过、在发声、在忙……
//! 同一棵树里的进程按同一份判断决定（各自只多一项：自己是否在输出声音）。
//!
//! 每轮按顺序：每个程序自己的情况、谁是重点程序 → 正在用的程序需要多少处理器（需求）→
//! 后台谁在忙、该让路 → 让到副域的程序分到哪个核心域。最后由 [`Scheduler::desired`] 得出每个进程应有的状态。

use std::collections::{HashMap, HashSet};

use super::decisions::Verdict;
use super::desired::Desired;
use super::policy;
use super::program::Program;
use super::trackers::{Demand, DemandSignals, Pressure};
use super::{ScheduleInput, Scheduler};
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::model::{Millis, Pid, PriorityClass, ProcessKey, Snapshot};
use crate::prefs::AppPreference;

impl Scheduler {
    /// 每个程序（进程树）此刻的情况。同一棵树里的进程一起判断，每轮每个程序只判断一次。
    pub(super) fn judge_programs(&mut self, input: &ScheduleInput) -> HashMap<Pid, Program> {
        let (classes, snap) = (input.classes, input.snapshot);
        let now = snap.at;
        let gaming = self.games.any_active();
        let elevated = self.self_elevated_roots(classes, snap);
        let boosted = self.boosts.roots(classes);
        let roots: HashSet<Pid> = classes.values().map(|c| c.root_pid).collect();
        let mut programs = HashMap::with_capacity(roots.len());
        // 第一遍：每个程序自己的情况，以及谁是重点程序。
        for root_pid in roots {
            let (Some(root_class), Some(root)) = (classes.get(&root_pid), self.load.root(root_pid))
            else {
                continue;
            };
            let mut program = Program {
                root,
                active_game: self.game_roots.contains(&root_pid),
                gaming,
                focus: None,
                focusing: false,
                efficiency: None,
                recently_foreground: self.foreground.recently_foreground(root_pid),
                boosted: boosted.contains(&root_pid),
                audible: root_class.plays_audio || self.audio.audible(root_pid),
                self_elevated: elevated.contains(&root_pid),
                busy: false,
                busy_with_audio: false,
                settling: false,
                yield_fully: false,
            };
            program.focus = self.focus_of(root_class, &program, now);
            programs.insert(root_pid, program);
        }
        // 有重点程序：直到最后一个重点程序结束一会儿；游戏按整个游戏会话算（一局结束、选英雄、
        // 再进下一局，中间不把一大批程序挪来挪去，也不放松后台的让路）。
        let focusing = self.focus.holding(now, self.cfg.focus.hold_ms)
            || self.games.in_session(now, self.cfg.game_session_hold_ms);
        self.assess_demand(input, &programs, focusing);
        // 第二遍：在不在忙（有重点程序时规则不同）。
        // 让路的程度：需求高（有重点程序、发热）或用电池时完整让路（让开主域、节能）；
        // 只是正在用的程序在忙时只降优先级，空闲的核心照样给它用。
        let yield_fully = self.pressure == Pressure::High || input.on_battery;
        let demand = Demand {
            now,
            pressure: self.pressure,
            idle_for: self.idle_for,
        };
        for (root_pid, program) in &mut programs {
            let Some(root_class) = classes.get(root_pid) else {
                continue;
            };
            program.focusing = focusing;
            program.yield_fully = yield_fully;
            let load = self.load.of(&program.root);
            let silent = Program {
                audible: false,
                ..*program
            };
            let in_use_besides_audio = policy::in_use(&silent.view(root_class, false));
            // 有重点程序时，发声只决定怎么让路（只让到小核），不决定要不要让路：声音停了直接按在忙让路，
            // 不会先恢复、过一会儿再让路。
            let in_use = if focusing {
                in_use_besides_audio
            } else {
                policy::in_use(&program.view(root_class, false))
            };
            program.busy = self
                .busy
                .observe(program.root, load, in_use, demand, self.cfg.busy);
            program.settling = self.busy.settling(&program.root, now, self.cfg.busy);
            // 发声的程序：只在有重点程序、除了发声之外没在用时，才让到小核。
            program.busy_with_audio = self.busy_with_audio.observe(
                program.root,
                load,
                !program.audible || !focusing || in_use_besides_audio,
                demand,
                self.cfg.busy_with_audio,
            );
        }
        self.place(classes, &mut programs);
        programs
    }

    /// 程序是不是重点程序；是的话，按它需要的线程算出它的核心范围（见 `trackers::focus`）。
    /// 还在等待确认的游戏先放到大核，但不算重点程序：确认之前不让别的程序让路。
    /// 用户设为"低优先级"的程序（临时全速的除外）不会成为重点程序：它在前台再忙也留在小核，
    /// 也不让别的程序为它让路。
    fn focus_of(
        &mut self,
        class: &Classification,
        program: &Program,
        now: Millis,
    ) -> Option<CoreSet> {
        let root = program.root;
        let pending_game = self.pending_game_roots.contains(&root.pid);
        let marked_low = class.preference == AppPreference::Background && !program.boosted;
        let directly_used = !marked_low
            && (class.in_foreground_tree
                || program.boosted
                || class.preference == AppPreference::AlwaysFast);
        let focused = class.is_schedulable()
            && (pending_game
                || self.focus.observe(
                    root,
                    program.active_game,
                    directly_used,
                    self.load.recent(&root),
                    self.cfg.focus,
                    now,
                ));
        if !focused {
            self.footprint.forget(&root);
            return None;
        }
        let (load, instant) = (self.load.of(&root).core_pct, self.load.instant(&root));
        Some(
            self.footprint
                .cores_for(root, load, instant, self.cfg.cores, now),
        )
    }

    /// 此刻的需求（见 `trackers::demand`）：只看正在用的程序需要多少处理器，后台自己把整机跑满不算。
    fn assess_demand(
        &mut self,
        input: &ScheduleInput,
        programs: &HashMap<Pid, Program>,
        focusing: bool,
    ) {
        let now = input.snapshot.at;
        let in_use_core_pct = programs
            .iter()
            .filter(|(pid, p)| {
                input
                    .classes
                    .get(pid)
                    .is_some_and(|c| policy::in_use(&p.view(c, false)))
            })
            .map(|(_, p)| self.load.instant(&p.root))
            .sum();
        let signals = DemandSignals {
            focused: focusing,
            hot: input.hot,
            on_battery: input.on_battery,
            in_use_core_pct,
            system_pct: input.activity.system_cpu_pct(),
        };
        self.pressure = self.demand.update(signals, now, self.cfg.demand);
        self.idle_for = self.demand.idle_for(now);
    }

    /// 让到副域的程序按核心域分配（见 `placement`）。重点程序多用了副域时，那些域留给它。
    fn place(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        programs: &mut HashMap<Pid, Program>,
    ) {
        let all = self.cfg.cores.all_mask();
        let reserved = programs
            .values()
            .filter_map(|p| match p.focus? {
                CoreSet::Domains(m) => Some(m),
                CoreSet::All => Some(all),
                _ => None,
            })
            .fold(0, |r, m| r | m)
            & !1;
        let wants: Vec<(ProcessKey, f32)> = programs
            .iter()
            .filter_map(|(pid, p)| {
                let class = classes.get(pid)?;
                let (desired, _) = policy::decide(&p.view(class, false));
                desired
                    .cores
                    .is_efficiency()
                    .then(|| (p.root, self.load.of(&p.root).core_pct))
            })
            .collect();
        let placed = self.placer.place(&wants, self.cfg.cores, reserved);
        for p in programs.values_mut() {
            p.efficiency = placed.get(&p.root).copied();
        }
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

    /// 进程应有的状态与程序的决定：策略的结论，让到小核的换成分到的那组小核；处理器不支持的核心范围不限制。
    pub(super) fn desired(
        &self,
        class: &Classification,
        program: &Program,
        emitting: bool,
    ) -> (Desired, Verdict) {
        let (mut desired, verdict) = policy::decide(&program.view(class, emitting));
        if desired.cores == CoreSet::Efficiency
            && let Some(set) = program.efficiency
        {
            desired.cores = set;
        }
        if !self.cfg.cores.allows(desired.cores) {
            desired.cores = CoreSet::All;
        }
        (desired, verdict)
    }
}
