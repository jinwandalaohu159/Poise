//! 智能调度：前台提速、后台按需让路、游戏模式、临时全速。
//!
//! 对外只有 [`Scheduler`]：每个采样周期调用一次 [`Scheduler::plan`]，
//! 把动作交给安全执行器后用 [`Scheduler::report`] 回报结果。

mod policy;
mod reconcile;
mod trackers;

use std::collections::{HashMap, HashSet};

use crate::action::{EcoQos, ExecError, ExecOutcome};
use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, SessionState, Snapshot};
use crate::prefs::Mode;

pub use policy::{Desired, PolicyConfig};
pub use reconcile::PlannedAction;

use policy::{PolicyContext, ProcessView};
use reconcile::Reconciler;
use trackers::{ForegroundTracker, GameTracker, TempBoosts};

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    pub policy: PolicyConfig,
    /// 失去前台后延迟多久再降级。
    pub debounce_ms: Millis,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            policy: PolicyConfig::default(),
            debounce_ms: 5_000,
        }
    }
}

pub struct ScheduleInput<'a> {
    pub snapshot: &'a Snapshot,
    pub session: &'a SessionState,
    pub classes: &'a HashMap<Pid, Classification>,
    pub activity: &'a ActivityTracker,
    pub user_mode: Mode,
    pub auto_full_for_games: bool,
}

/// 正在运行的游戏（用于界面显示"游戏中 · 英雄联盟"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInfo {
    pub pid: Pid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulePlan {
    /// 实际生效的模式（玩游戏时可能被自动切到全力）。
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
    pub foreground_busy: bool,
    pub actions: Vec<PlannedAction>,
}

#[derive(Default)]
pub struct Scheduler {
    cfg: ScheduleConfig,
    foreground: ForegroundTracker,
    games: GameTracker,
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
            .update(now, input.classes, self.cfg.debounce_ms);
        self.games.update(snap, input.classes);
        self.boosts.expire(now);
        let alive: HashSet<ProcessKey> = snap.processes.iter().map(|p| p.facts.key).collect();
        self.reconciler.retain_alive(&alive);

        let game_active = self.games.any_active();
        let effective_mode = if game_active && input.auto_full_for_games {
            Mode::Full
        } else {
            input.user_mode
        };
        let foreground_busy = game_active
            || input.session.fullscreen
            || self.foreground_load(input) >= self.cfg.policy.foreground_busy_core_pct;
        let ctx = PolicyContext {
            mode: effective_mode,
            foreground_busy,
        };

        let mut actions = Vec::new();
        for p in &snap.processes {
            let Some(class) = input.classes.get(&p.facts.key.pid) else {
                continue;
            };
            let key = p.facts.key;
            let view = ProcessView {
                class,
                usage: input.activity.usage(&key),
                active_game: self.games.is_active(&key),
                recently_foreground: self.foreground.recently_foreground(class.root_pid),
                boosted: self.boosts.is_boosted(&key, class.root_pid),
                yielding: self.reconciler.current(&key).eco == EcoQos::On,
            };
            let desired = policy::decide(&self.cfg.policy, &ctx, &view);
            actions.extend(self.reconciler.diff(key, p.facts.priority, desired));
        }

        SchedulePlan {
            effective_mode,
            game: self.current_game(snap),
            foreground_busy,
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

    /// 撤销所有调整（服务停止时调用）。
    pub fn release_all(&self) -> Vec<PlannedAction> {
        self.reconciler.restore_all()
    }

    pub fn adjusted_count(&self) -> usize {
        self.reconciler.applied_count()
    }

    fn foreground_load(&self, input: &ScheduleInput) -> f32 {
        input
            .snapshot
            .processes
            .iter()
            .filter(|p| {
                input
                    .classes
                    .get(&p.facts.key.pid)
                    .is_some_and(|c| c.in_foreground_tree)
            })
            .map(|p| input.activity.usage(&p.facts.key).core_pct)
            .sum()
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
mod tests {
    use super::*;
    use crate::action::{Action, CoreSet};
    use crate::classify::Role;
    use crate::model::{PriorityClass, ProcessFacts, ProcessSample};
    use crate::prefs::AppPreference;

    fn sample(pid: Pid, name: &str) -> ProcessSample {
        ProcessSample {
            facts: ProcessFacts {
                key: ProcessKey { pid, created: 1 },
                name: name.into(),
                session_id: 1,
                priority: Some(PriorityClass::Normal),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn class(root: Pid, fg: bool, game: bool) -> Classification {
        Classification {
            role: Role::UserApp,
            root_pid: root,
            in_foreground_tree: fg,
            has_visible_window: true,
            tree_has_window: false,
            plays_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: game,
            anti_cheat_present: false,
            restartable_service: None,
        }
    }

    fn run(
        s: &mut Scheduler,
        at: Millis,
        classes: &HashMap<Pid, Classification>,
        mode: Mode,
    ) -> SchedulePlan {
        let snap = Snapshot {
            at,
            processes: vec![sample(1, "game.exe"), sample(2, "chrome.exe")],
            ..Default::default()
        };
        let plan = s.plan(&ScheduleInput {
            snapshot: &snap,
            session: &SessionState::default(),
            classes,
            activity: &ActivityTracker::default(),
            user_mode: mode,
            auto_full_for_games: true,
        });
        for a in &plan.actions {
            s.report(a, &Ok(ExecOutcome::default()));
        }
        plan
    }

    #[test]
    fn game_switches_to_full_and_stays_active_when_alt_tabbed() {
        let mut s = Scheduler::default();
        let mut classes = HashMap::from([(1, class(1, true, true)), (2, class(2, false, false))]);
        let plan = run(&mut s, 0, &classes, Mode::Balanced);
        assert_eq!(plan.effective_mode, Mode::Full);
        assert_eq!(plan.game.as_ref().map(|g| g.pid), Some(1));
        assert!(plan.actions.contains(&PlannedAction {
            key: ProcessKey { pid: 1, created: 1 },
            action: Action::SetPriority {
                pid: 1,
                class: PriorityClass::High
            },
            original_priority: PriorityClass::Normal,
        }));

        // 切到浏览器：游戏仍被记住，模式仍是全力。
        classes.insert(1, class(1, false, false));
        classes.insert(2, class(2, true, false));
        let plan = run(&mut s, 1000, &classes, Mode::Balanced);
        assert_eq!(plan.effective_mode, Mode::Full);
    }

    #[test]
    fn switching_foreground_demotes_only_after_debounce() {
        let mut s = Scheduler::default();
        let fg1 = HashMap::from([(1, class(1, true, false)), (2, class(2, false, false))]);
        run(&mut s, 0, &fg1, Mode::Balanced);
        let fg2 = HashMap::from([(1, class(1, false, false)), (2, class(2, true, false))]);
        let plan = run(&mut s, 1000, &fg2, Mode::Balanced);
        // 防抖期内，进程 1 的提速不会被撤销。
        assert!(!plan.actions.iter().any(|a| a.key.pid == 1));
        let plan = run(&mut s, 7000, &fg2, Mode::Balanced);
        assert!(plan.actions.iter().any(|a| a.key.pid == 1
            && a.action
                == Action::SetEcoQos {
                    pid: 1,
                    state: EcoQos::Default
                }));
    }

    #[test]
    fn release_all_restores_every_adjusted_process() {
        let mut s = Scheduler::default();
        let classes = HashMap::from([(1, class(1, true, false)), (2, class(2, false, false))]);
        run(&mut s, 0, &classes, Mode::Balanced);
        assert!(s.adjusted_count() > 0);
        let restore = s.release_all();
        assert!(restore.iter().all(|a| matches!(
            a.action,
            Action::SetPriority {
                class: PriorityClass::Normal,
                ..
            } | Action::SetEcoQos {
                state: EcoQos::Default,
                ..
            } | Action::SetCoreSet {
                set: CoreSet::All,
                ..
            }
        )));
    }
}
