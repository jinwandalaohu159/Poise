//! 智能调度：按程序类别和前台/后台状态分配核心、优先级和节能；游戏模式；临时全速。
//!
//! 对外只有 [`Scheduler`]：每个采样周期调用一次 [`Scheduler::plan`]，
//! 把动作交给安全执行器后用 [`Scheduler::report`] 回报结果。

mod policy;
mod reconcile;
mod trackers;

use std::collections::{HashMap, HashSet};

use crate::action::{ActionKind, CoreSet, ExecError, ExecOutcome};
use crate::classify::Classification;
use crate::model::{Millis, Pid, PriorityClass, ProcessKey, ProcessSample, Snapshot};
use crate::prefs::Mode;

pub use policy::Desired;
pub use reconcile::{PlannedAction, ProcessSchedule};

use policy::ProcessView;
use reconcile::Reconciler;
use trackers::{ForegroundTracker, GameTracker, TempBoosts};

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// 失去前台后延迟多久再降级。
    pub debounce_ms: Millis,
    /// 是否是大小核处理器。不是时不限制核心，只用优先级和节能区分前后台。
    pub hybrid: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 5_000,
            hybrid: true,
        }
    }
}

pub struct ScheduleInput<'a> {
    pub snapshot: &'a Snapshot,
    pub classes: &'a HashMap<Pid, Classification>,
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
                self.desired(class, &key)
            } else {
                Desired::NEUTRAL
            };
            let inherited_low = self.inherited_low(p, &by_pid);
            actions.extend(
                self.reconciler
                    .diff(key, p.facts.priority, inherited_low, desired),
            );
        }
        // 先提速（新的前台程序立即拿到性能），再让路和恢复。
        actions.sort_by_key(|a| a.action.kind(a.original_priority) != ActionKind::Boost);

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

    /// 撤销所有调整（服务停止时调用）。
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

    fn desired(&self, class: &Classification, key: &ProcessKey) -> Desired {
        let mut desired = policy::decide(&ProcessView {
            class,
            active_game: self.games.is_active(key),
            recently_foreground: self.foreground.recently_foreground(class.root_pid),
            boosted: self.boosts.is_boosted(key, class.root_pid),
        });
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
mod tests {
    use super::*;
    use crate::action::{Action, CoreSet, EcoQos};
    use crate::classify::{Role, ScheduleScope};
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
            schedule: ScheduleScope::App,
        }
    }

    /// 采样里的优先级反映 Poise 已经设置的值（与真实系统一致）。
    fn snapshot(s: &Scheduler, at: Millis) -> Snapshot {
        let mut processes = vec![sample(1, "game.exe"), sample(2, "chrome.exe")];
        for p in &mut processes {
            if let ProcessSchedule::Adjusted(d) = s.state(&p.facts.key) {
                p.facts.priority = d.priority.or(p.facts.priority);
            }
        }
        Snapshot {
            at,
            processes,
            ..Default::default()
        }
    }

    fn run(
        s: &mut Scheduler,
        at: Millis,
        classes: &HashMap<Pid, Classification>,
        mode: Mode,
    ) -> SchedulePlan {
        let snap = snapshot(s, at);
        let plan = s.plan(&ScheduleInput {
            snapshot: &snap,
            classes,
            enabled: true,
            user_mode: mode,
            game_mode: Some(Mode::Full),
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
                == Action::SetCoreSet {
                    pid: 1,
                    set: CoreSet::Efficiency
                }));
    }

    #[test]
    fn turning_smart_scheduling_off_restores_everything() {
        let mut s = Scheduler::default();
        let classes = HashMap::from([(1, class(1, true, false)), (2, class(2, false, false))]);
        run(&mut s, 0, &classes, Mode::Balanced);
        assert_eq!(s.adjusted_count(), 2);
        let snap = snapshot(&s, 1000);
        let plan = s.plan(&ScheduleInput {
            snapshot: &snap,
            classes: &classes,
            enabled: false,
            user_mode: Mode::Balanced,
            game_mode: None,
        });
        for a in &plan.actions {
            s.report(a, &Ok(ExecOutcome::default()));
        }
        assert_eq!(s.adjusted_count(), 0);
    }

    #[test]
    fn children_that_inherited_low_priority_are_restored_to_normal() {
        let mut s = Scheduler::default();
        let classes = HashMap::from([(2, class(2, false, false)), (3, class(2, false, false))]);
        let mut parent = sample(2, "app.exe");
        let snap = |processes| Snapshot {
            at: 0,
            processes,
            ..Default::default()
        };
        let apply = |s: &mut Scheduler, snap: &Snapshot| {
            let plan = s.plan(&ScheduleInput {
                snapshot: snap,
                classes: &classes,
                enabled: true,
                user_mode: Mode::Balanced,
                game_mode: None,
            });
            for a in &plan.actions {
                s.report(a, &Ok(ExecOutcome::default()));
            }
        };
        apply(&mut s, &snap(vec![parent.clone()]));
        // 父进程已被降低；之后新建的子进程继承了"低于正常"。
        parent.facts.priority = Some(PriorityClass::BelowNormal);
        let mut child = sample(3, "helper.exe");
        child.facts.parent_pid = Some(2);
        child.facts.key.created = 2;
        child.facts.priority = Some(PriorityClass::BelowNormal);
        apply(&mut s, &snap(vec![parent, child]));
        let restore = s.release_all();
        assert!(restore.iter().any(|a| a.key.pid == 3
            && a.action
                == Action::SetPriority {
                    pid: 3,
                    class: PriorityClass::Normal
                }));
    }

    #[test]
    fn boosts_come_before_demotions() {
        let mut s = Scheduler::new(ScheduleConfig {
            debounce_ms: 0,
            ..Default::default()
        });
        run(
            &mut s,
            0,
            &HashMap::from([(1, class(1, true, false)), (2, class(2, false, false))]),
            Mode::Balanced,
        );
        // 前台从 1 切到 2（不防抖）：同一轮里 2 提速、1 降级，提速排在前面。
        let swapped = HashMap::from([(1, class(1, false, false)), (2, class(2, true, false))]);
        let plan = run(&mut s, 10_000, &swapped, Mode::Balanced);
        let first_demote = plan
            .actions
            .iter()
            .position(|a| a.key.pid == 1)
            .expect("进程 1 降级");
        assert!(plan.actions[..first_demote].iter().all(|a| a.key.pid == 2));
        assert!(plan.actions.iter().any(|a| a.key.pid == 2));
    }

    #[test]
    fn non_hybrid_cpus_never_restrict_cores() {
        let mut s = Scheduler::new(ScheduleConfig {
            hybrid: false,
            ..Default::default()
        });
        let classes = HashMap::from([(1, class(1, false, false)), (2, class(2, false, false))]);
        let plan = run(&mut s, 0, &classes, Mode::Balanced);
        assert!(
            !plan
                .actions
                .iter()
                .any(|a| matches!(a.action, Action::SetCoreSet { .. }))
        );
        assert!(!plan.actions.is_empty());
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
