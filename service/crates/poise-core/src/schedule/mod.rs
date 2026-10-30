//! 智能调度：重点程序（正在玩的游戏、正在用又在持续干重活的程序）按需要的线程占用核心，其余没在用的
//! 程序让到副域；后台持续占用处理器或磁盘的程序按需求分档让路；正在用、正在发声的程序不动；标记的游戏
//! 提高一级；游戏模式；临时全速。规则见 [`policy`]。
//!
//! 对外只有 [`Scheduler`]：每个采样周期调用一次 [`Scheduler::plan`]，
//! 把动作交给安全执行器后用 [`Scheduler::report`] 回报结果。
//!
//! 每一轮：`trackers` 跨周期观察（前台、游戏、发声、负载、需求、忙碌、重点程序）→ `judge` 判断每个程序
//! （进程树，见 `program`）、算出需求、给让到副域的程序分配核心域（`placement`）→ `policy` 决定每个进程
//! 应有的状态（各种状态见 `desired`）→ `reconcile` 与已应用的比较，得到动作 → `explain` 解释决定，
//! `decisions` 记下变化供日志。参数与处理器的核心域在 `config`。

mod config;
mod decisions;
mod desired;
mod explain;
mod judge;
mod leftovers;
mod placement;
mod policy;
mod program;
mod reconcile;
mod trackers;

use std::collections::{HashMap, HashSet};

use crate::action::{ActionKind, ExecError, ExecOutcome};
use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, PriorityClass, ProcessKey, ProcessSample, Snapshot};
use crate::prefs::Mode;

pub use config::{CoreLayout, ScheduleConfig};
pub use decisions::{Cause, Decision, ProgramState, Verdict};
pub use desired::Desired;
pub use leftovers::Adjusted;
pub use reconcile::{PlannedAction, ProcessSchedule};

use decisions::DecisionLog;
use placement::GroupPlacer;
use program::Program;
use reconcile::Reconciler;
use trackers::{
    AudioTracker, BusyTracker, DemandTracker, FocusTracker, Footprint, ForegroundTracker,
    GameTracker, TempBoosts, TreeLoad,
};
pub use trackers::{BusyRule, DemandRule, FocusRule, ForegroundHold, LoadSmoothing, Pressure};

pub struct ScheduleInput<'a> {
    pub snapshot: &'a Snapshot,
    pub classes: &'a HashMap<Pid, Classification>,
    /// 各进程最近的处理器占用。
    pub activity: &'a ActivityTracker,
    /// "智能调度"开关。关闭时撤销全部进程调整，模式切换照常。
    pub enabled: bool,
    /// 用户的模式（"自动"已换算成具体模式）。
    pub user_mode: Mode,
    /// 玩游戏时切换到的模式；None 表示不切换。
    pub game_mode: Option<Mode>,
    pub on_battery: bool,
    /// 处理器接近温度上限（上一轮的判断）。
    pub hot: bool,
}

/// 正在运行的游戏（用于界面显示"游戏中 · 英雄联盟"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInfo {
    pub pid: Pid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulePlan {
    /// 实际生效的模式（玩游戏时可能被切到全力或极限模式）。
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
    /// 有重点程序（或刚结束不久，见 `trackers::focus`）。
    pub focused: bool,
    pub actions: Vec<PlannedAction>,
    /// 和上一轮相比，需求和对各程序的决定有什么变化（供日志记录）。
    pub decisions: Vec<Decision>,
}

#[derive(Default)]
pub struct Scheduler {
    cfg: ScheduleConfig,
    foreground: ForegroundTracker,
    games: GameTracker,
    /// 正在玩的游戏所在的进程树（每轮更新）。
    game_roots: HashSet<Pid>,
    /// 像游戏、还在等待确认的进程树：先放到大核，不等确认（每轮更新）。
    pending_game_roots: HashSet<Pid>,
    focus: FocusTracker,
    footprint: Footprint,
    placer: GroupPlacer,
    audio: AudioTracker,
    load: TreeLoad,
    demand: DemandTracker,
    /// 本轮的资源需求，以及已经连续多久没有需求。
    pressure: Pressure,
    idle_for: Millis,
    busy: BusyTracker,
    /// 有重点程序时，发声又在忙的程序（只让到小核）。
    busy_with_audio: BusyTracker,
    boosts: TempBoosts,
    decisions: DecisionLog,
    reconciler: Reconciler,
}

impl Scheduler {
    pub fn new(cfg: ScheduleConfig) -> Self {
        Self {
            cfg,
            ..Default::default()
        }
    }

    pub fn plan(&mut self, input: &ScheduleInput) -> SchedulePlan {
        let effective_mode = self.observe(input);
        let programs = if input.enabled {
            self.judge_programs(input)
        } else {
            HashMap::new()
        };
        let focused = programs.values().any(|p| p.focusing);
        let actions = self.diff(input, &programs);
        let decisions = self.record_decisions(input.classes, &programs);
        SchedulePlan {
            effective_mode,
            game: self.current_game(input.snapshot),
            focused,
            actions,
            decisions,
        }
    }

    pub fn report(&mut self, planned: &PlannedAction, result: &Result<ExecOutcome, ExecError>) {
        self.reconciler.report(planned, result);
    }

    /// 临时全速：作用于该进程所在的整个进程树。
    pub fn boost(&mut self, key: ProcessKey, until: Millis) {
        self.boosts.add(key, until);
    }

    /// 忘掉前台、游戏、忙碌、临时全速等跟踪状态，从头开始（Poise 暂停时，撤销调整之后调用）。
    /// 已应用调整的记录保留：没能撤销的调整之后还会再撤销。
    pub fn reset_tracking(&mut self) {
        let reconciler = std::mem::take(&mut self.reconciler);
        *self = Self {
            reconciler,
            ..Self::new(self.cfg)
        };
    }

    /// 正在玩的游戏。它们各自是独立的程序（不归到启动它的客户端名下）。
    pub fn active_games(&self) -> Vec<Pid> {
        self.games.active_keys().map(|k| k.pid).collect()
    }

    /// 撤销所有调整（服务停止、Poise 退出时调用）。
    pub fn release_all(&self) -> Vec<PlannedAction> {
        self.reconciler.restore_all()
    }

    pub fn adjusted_count(&self) -> usize {
        self.reconciler.applied_count()
    }

    /// 所有已调整的进程（服务把它存下来，见 [`Adjusted`]）。
    pub fn adjusted(&self) -> Vec<Adjusted> {
        self.reconciler.adjusted()
    }

    /// 恢复上次运行留下的调整所需的动作（只对还在运行的同一个进程，见 [`Adjusted`]）。
    pub fn restore_leftovers(&self, leftovers: &[Adjusted], snap: &Snapshot) -> Vec<PlannedAction> {
        leftovers
            .iter()
            .filter(|l| snap.find(l.key.pid).is_some_and(|p| p.facts.key == l.key))
            .flat_map(leftovers::restore_actions)
            .collect()
    }

    /// 一个进程当前的调度状态（程序偏好页显示）。
    pub fn state(&self, key: &ProcessKey) -> ProcessSchedule {
        self.reconciler.state(key)
    }

    /// 更新各项跟踪（前台、游戏、发声、临时全速、负载），返回实际生效的模式。需求要等判断完哪些程序
    /// 正在用之后才算（见 `program`）。
    fn observe(&mut self, input: &ScheduleInput) -> Mode {
        let snap = input.snapshot;
        let now = snap.at;
        self.foreground
            .update(now, input.classes, self.cfg.foreground_hold);
        self.games
            .update(snap, input.classes, now, self.cfg.game_confirm_ms);
        self.audio
            .update(snap, input.classes, now, self.cfg.audio_hold_ms);
        self.boosts.expire(now);
        let alive: HashSet<ProcessKey> = snap.processes.iter().map(|p| p.facts.key).collect();
        self.reconciler.retain_alive(&alive);
        self.focus.retain_alive(&alive);
        self.footprint.retain_alive(&alive);
        self.placer.retain_alive(&alive);
        if input.enabled {
            self.busy.retain_alive(&alive);
            self.busy_with_audio.retain_alive(&alive);
        } else {
            // 关闭期间不累计"在忙"：重新打开时从头判断，不会一打开就让一批程序让路。
            self.busy = BusyTracker::default();
            self.busy_with_audio = BusyTracker::default();
        }

        self.game_roots = self
            .games
            .active_keys()
            .filter_map(|k| input.classes.get(&k.pid))
            .map(|c| c.root_pid)
            .collect();
        // 只认自己就是一个程序的候选（游戏本体）：挂在启动器名下的不算，不把启动器整棵树搬到大核。
        self.pending_game_roots = self
            .games
            .pending_keys()
            .filter(|k| {
                input
                    .classes
                    .get(&k.pid)
                    .is_some_and(|c| c.root_pid == k.pid)
            })
            .map(|k| k.pid)
            .collect();
        self.load.update(
            snap,
            input.classes,
            input.activity,
            now,
            self.cfg.load_smoothing,
        );
        match input.game_mode {
            Some(game) if self.games.in_session(now, self.cfg.game_session_hold_ms) => game,
            _ => input.user_mode,
        }
    }

    /// 每个进程期望的状态与已应用的比较，得到要执行的动作。先恢复（回到前台的程序立即拿回性能），再让路。
    fn diff(
        &mut self,
        input: &ScheduleInput,
        programs: &HashMap<Pid, Program>,
    ) -> Vec<PlannedAction> {
        let snap = input.snapshot;
        self.reconciler.begin(snap.at);
        let by_pid = snap.by_pid();
        let mut actions = Vec::new();
        for p in &snap.processes {
            let Some(class) = input.classes.get(&p.facts.key.pid) else {
                continue;
            };
            let key = p.facts.key;
            let desired = match programs.get(&class.root_pid) {
                Some(program) => {
                    let emitting = class.emits_audio || self.audio.emitting(&key);
                    self.desired(class, program, emitting).0
                }
                None => Desired::NEUTRAL,
            };
            let inherited_low = self.inherited_low(p, &by_pid);
            actions.extend(
                self.reconciler
                    .diff(key, p.facts.priority, inherited_low, desired),
            );
        }
        actions.sort_by_key(|a| a.action.kind(a.original_priority) == ActionKind::Demote);
        actions
    }

    /// 父进程被 Poise 降为"低于正常"后新建的子进程，会继承这个优先级（Windows 的规则）。
    fn inherited_low(&self, p: &ProcessSample, by_pid: &HashMap<Pid, &ProcessSample>) -> bool {
        p.facts.priority == Some(PriorityClass::BelowNormal)
            && p.facts
                .parent_pid
                .and_then(|pp| by_pid.get(&pp))
                .is_some_and(|parent| {
                    parent.is_parent_of(p) && self.reconciler.lowered(&parent.facts.key)
                })
    }

    fn current_game(&self, snap: &Snapshot) -> Option<GameInfo> {
        self.games
            .active_keys()
            .filter_map(|k| snap.processes.iter().find(|p| p.facts.key == *k))
            .min_by_key(|p| p.facts.key.created)
            .map(|p| GameInfo {
                pid: p.facts.key.pid,
                name: p.facts.friendly_name(),
            })
    }
}

#[cfg(test)]
mod sim;
