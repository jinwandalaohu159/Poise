//! 智能调度：只让路、不提速——在后台持续占用处理器的程序挪到小核并节能，正在用的程序不动；
//! 游戏模式；临时全速。规则见 [`policy`]。
//!
//! 对外只有 [`Scheduler`]：每个采样周期调用一次 [`Scheduler::plan`]，
//! 把动作交给安全执行器后用 [`Scheduler::report`] 回报结果。

mod policy;
mod reconcile;
mod trackers;

use std::collections::{HashMap, HashSet};

use crate::action::{ActionKind, CoreSet, ExecError, ExecOutcome};
use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, PriorityClass, ProcessKey, ProcessSample, Snapshot};
use crate::prefs::Mode;

pub use policy::Desired;
pub use reconcile::{PlannedAction, ProcessSchedule};

use policy::ProcessView;
use reconcile::Reconciler;
pub use trackers::{BusyRule, ForegroundHold};
use trackers::{BusyTracker, ForegroundTracker, GameTracker, TempBoosts};

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// 失去前台后多久内仍算"正在用"、最多保持几个程序。
    pub foreground_hold: ForegroundHold,
    /// 后台程序怎样算"在忙"、要让路。
    pub busy: BusyRule,
    /// 自动识别的游戏（全屏且在渲染）要持续多久才确认为"正在玩"。
    pub game_confirm_ms: Millis,
    /// 是否是大小核处理器。不是时不限制核心，只用优先级和节能区分前后台。
    pub hybrid: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            foreground_hold: ForegroundHold {
                ms: 5 * 60_000,
                max_apps: 3,
            },
            busy: BusyRule {
                core_pct: 10.0,
                after_ms: 10_000,
            },
            game_confirm_ms: 6_000,
            hybrid: true,
        }
    }
}

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
    pub actions: Vec<PlannedAction>,
}

#[derive(Default)]
pub struct Scheduler {
    cfg: ScheduleConfig,
    foreground: ForegroundTracker,
    games: GameTracker,
    busy: BusyTracker,
    boosts: TempBoosts,
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
        let snap = input.snapshot;
        let now = snap.at;
        self.foreground
            .update(now, input.classes, self.cfg.foreground_hold);
        self.games
            .update(snap, input.classes, now, self.cfg.game_confirm_ms);
        self.boosts.expire(now);
        let alive: HashSet<ProcessKey> = snap.processes.iter().map(|p| p.facts.key).collect();
        self.reconciler.retain_alive(&alive);
        if input.enabled {
            self.busy.retain_alive(&alive);
        } else {
            // 关闭期间不累计"在忙"：重新打开时从头判断，不会一打开就让一批程序让路。
            self.busy = BusyTracker::default();
        }

        let game_active = self.games.any_active();
        let effective_mode = match input.game_mode {
            Some(game) if game_active => game,
            _ => input.user_mode,
        };

        self.reconciler.begin(now);
        let by_pid = snap.by_pid();
        let mut actions = Vec::new();
        for p in &snap.processes {
            let Some(class) = input.classes.get(&p.facts.key.pid) else {
                continue;
            };
            let key = p.facts.key;
            let desired = if input.enabled {
                let core_pct = input.activity.usage(&key).core_pct;
                self.desired(class, key, core_pct, now)
            } else {
                Desired::NEUTRAL
            };
            let inherited_low = self.inherited_low(p, &by_pid);
            actions.extend(
                self.reconciler
                    .diff(key, p.facts.priority, inherited_low, desired),
            );
        }
        // 先恢复（回到前台的程序立即拿回性能），再让路。
        actions.sort_by_key(|a| a.action.kind(a.original_priority) == ActionKind::Demote);

        SchedulePlan {
            effective_mode,
            game: self.current_game(snap),
            actions,
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

    /// 一个进程当前的调度状态（程序偏好页显示）。
    pub fn state(&self, key: &ProcessKey) -> ProcessSchedule {
        self.reconciler.state(key)
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

    fn desired(
        &mut self,
        class: &Classification,
        key: ProcessKey,
        core_pct: f32,
        now: Millis,
    ) -> Desired {
        let mut view = ProcessView {
            class,
            active_game: self.games.is_active(&key),
            recently_foreground: self.foreground.recently_foreground(class.root_pid),
            boosted: self.boosts.is_boosted(&key, class.root_pid),
            busy: false,
        };
        view.busy = self
            .busy
            .observe(key, core_pct, policy::in_use(&view), now, self.cfg.busy);
        let mut desired = policy::decide(&view);
        if !self.cfg.hybrid {
            desired.cores = CoreSet::All;
        }
        desired
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
