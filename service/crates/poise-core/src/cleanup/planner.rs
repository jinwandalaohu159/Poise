//! 清理计划：把"发现的问题"变成"可以执行的步骤"，按进程类型选动作。

use std::collections::{HashMap, HashSet};

use crate::action::Action;
use crate::classify::{Classification, Role};
use crate::model::{Pid, ProcessKey, ProcessSample, Snapshot};
use crate::safety::{SafetyGuard, Trigger};

use super::growth::{Grown, GrowthMetric};
use super::leak::{LeakMetric, LeakSuspect};

#[derive(Debug, Clone, Copy)]
pub struct CleanupConfig {
    /// 服务及其辅助进程超过这个私有内存，即使没有历史趋势也视为异常。
    pub oversized_service_bytes: u64,
    /// 合成器超过这个私有内存视为异常。
    pub oversized_compositor_bytes: u64,
    /// 有窗口的程序超过这个私有内存时，提示用户手动关掉重开。
    pub heavy_window_bytes: u64,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        const MB: u64 = 1024 * 1024;
        Self {
            oversized_service_bytes: 768 * MB,
            oversized_compositor_bytes: 1024 * MB,
            heavy_window_bytes: 1536 * MB,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupReason {
    Leak(LeakMetric),
    /// 与进程自身稳定后的基线相比增长了数倍。
    Grown(GrowthMetric),
    Oversized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupStep {
    pub key: ProcessKey,
    pub action: Action,
    /// 会造成短暂闪屏或断音。
    pub disruptive: bool,
    pub reason: CleanupReason,
    pub name: String,
    /// 处理后预计归还的物理内存（专用工作集）。
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Advisory {
    /// 有窗口的程序占用很多，建议用户自己关掉重开。
    HeavyApp {
        pid: Pid,
        name: String,
        memory_bytes: u64,
    },
    /// 内核内存池过大，只有真重启能解决。
    RestartRecommended { kernel_nonpaged: u64 },
    /// 自动清理跳过的、需要用户点按钮的项目数量。
    ManualPending { count: usize },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupPlan {
    pub run_now: Vec<CleanupStep>,
    /// 用户点了清理，但前台有游戏或全屏程序，等它退出再做。
    pub deferred: Vec<CleanupStep>,
    pub advisories: Vec<Advisory>,
}

impl CleanupPlan {
    pub fn is_empty(&self) -> bool {
        self.run_now.is_empty() && self.deferred.is_empty()
    }

    pub fn expected_free(&self) -> u64 {
        self.run_now.iter().map(|s| s.memory_bytes).sum()
    }

    /// 包括延后项在内，这份计划总共能释放的内存。
    pub fn total_freeable(&self) -> u64 {
        self.expected_free() + self.deferred.iter().map(|s| s.memory_bytes).sum::<u64>()
    }
}

pub struct PlanInput<'a> {
    pub snapshot: &'a Snapshot,
    pub classes: &'a HashMap<Pid, Classification>,
    pub suspects: &'a [LeakSuspect],
    /// 与自身基线相比异常增长的进程（由 [`super::GrowthWatch`] 判断）。
    pub grown: &'a [Grown],
    pub trigger: Trigger,
    /// 前台有游戏或全屏程序。
    pub foreground_immersive: bool,
    /// 内核内存池需要真重启（由 [`super::KernelPoolWatch`] 按趋势判断）。
    pub restart_advised: bool,
}

pub fn plan(cfg: &CleanupConfig, input: &PlanInput) -> CleanupPlan {
    let mut plan = CleanupPlan::default();
    let suspects: HashMap<ProcessKey, CleanupReason> = input
        .grown
        .iter()
        .map(|g| (g.key, CleanupReason::Grown(g.change.metric)))
        // 两者都有时，按趋势得出的结论优先（后插入的覆盖前面的）。
        .chain(
            input
                .suspects
                .iter()
                .map(|s| (s.key, CleanupReason::Leak(s.metric))),
        )
        .collect();
    let mut seen_services = HashSet::new();
    let mut manual_pending = 0;

    for p in &input.snapshot.processes {
        let Some(c) = input.classes.get(&p.facts.key.pid) else {
            continue;
        };
        let Some(reason) = reason_for(cfg, p, c, &suspects) else {
            continue;
        };
        let Some((action, disruptive)) = action_for(p, c) else {
            if c.role == Role::UserApp && c.has_visible_window {
                plan.advisories.push(heavy_app(p));
            }
            continue;
        };
        if let Action::RestartService { service, .. } = &action
            && !seen_services.insert(service.to_lowercase())
        {
            continue;
        }
        let original = p.facts.priority.unwrap_or_default();
        // 计划阶段按"手动"检查，自动清理时再单独处理会打扰的项目。
        if SafetyGuard::check(&action, c, original, Trigger::Manual).is_err() {
            continue;
        }
        let step = CleanupStep {
            key: p.facts.key,
            action,
            disruptive,
            reason,
            name: p.facts.friendly_name(),
            memory_bytes: p.metrics.memory_bytes,
        };
        match (input.trigger, disruptive) {
            (Trigger::Auto, true) => manual_pending += 1,
            (Trigger::Auto, false) => {
                if SafetyGuard::check(&step.action, c, original, Trigger::Auto).is_ok() {
                    plan.run_now.push(step);
                }
            }
            (Trigger::Manual, true) if input.foreground_immersive => plan.deferred.push(step),
            (Trigger::Manual, _) => plan.run_now.push(step),
        }
    }

    add_heavy_window_hints(cfg, input, &mut plan);
    if input.restart_advised {
        plan.advisories.push(Advisory::RestartRecommended {
            kernel_nonpaged: input.snapshot.system.kernel_nonpaged,
        });
    }
    if manual_pending > 0 {
        plan.advisories.push(Advisory::ManualPending {
            count: manual_pending,
        });
    }
    plan.run_now.sort_by_key(|s| s.disruptive);
    plan
}

fn reason_for(
    cfg: &CleanupConfig,
    p: &ProcessSample,
    c: &Classification,
    suspects: &HashMap<ProcessKey, CleanupReason>,
) -> Option<CleanupReason> {
    if let Some(reason) = suspects.get(&p.facts.key) {
        return Some(*reason);
    }
    let private = p.metrics.private_bytes;
    let oversized = match c.role {
        Role::Compositor => private >= cfg.oversized_compositor_bytes,
        Role::ServiceChild { .. } | Role::ServiceHost => {
            c.restartable_service.is_some() && private >= cfg.oversized_service_bytes
        }
        _ => false,
    };
    oversized.then_some(CleanupReason::Oversized)
}

/// 按进程类型选择动作，并标记是否会造成短暂打扰。
fn action_for(p: &ProcessSample, c: &Classification) -> Option<(Action, bool)> {
    let pid = p.facts.key.pid;
    match &c.role {
        Role::Compositor => Some((Action::RecreateCompositor { pid }, true)),
        Role::ServiceChild { service } => Some((
            Action::RestartService {
                pid,
                service: service.clone(),
            },
            true,
        )),
        Role::ServiceHost => c.restartable_service.as_ref().map(|s| {
            (
                Action::RestartService {
                    pid,
                    service: s.clone(),
                },
                false,
            )
        }),
        Role::UserApp if !c.has_visible_window => Some((Action::RestartApp { pid }, false)),
        _ => None,
    }
}

fn heavy_app(p: &ProcessSample) -> Advisory {
    Advisory::HeavyApp {
        pid: p.facts.key.pid,
        name: p.facts.friendly_name(),
        memory_bytes: p.metrics.memory_bytes,
    }
}

/// 没有泄漏但占用特别大的窗口程序：最多提示两个，按占用排序。
fn add_heavy_window_hints(cfg: &CleanupConfig, input: &PlanInput, plan: &mut CleanupPlan) {
    let already: HashSet<Pid> = plan
        .advisories
        .iter()
        .filter_map(|a| match a {
            Advisory::HeavyApp { pid, .. } => Some(*pid),
            _ => None,
        })
        .collect();
    let mut heavy: Vec<&ProcessSample> = input
        .snapshot
        .processes
        .iter()
        .filter(|p| {
            input.classes.get(&p.facts.key.pid).is_some_and(|c| {
                c.role == Role::UserApp
                    && c.has_visible_window
                    && !c.in_foreground_tree
                    && !c.is_game
            }) && p.metrics.memory_bytes >= cfg.heavy_window_bytes
                && !already.contains(&p.facts.key.pid)
        })
        .collect();
    heavy.sort_by_key(|p| std::cmp::Reverse(p.metrics.memory_bytes));
    plan.advisories
        .extend(heavy.into_iter().take(2).map(heavy_app));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::ScheduleScope;
    use crate::model::{ProcessFacts, ProcessMetrics, ServiceFacts, SystemMetrics};
    use crate::prefs::AppPreference;

    const MB: u64 = 1024 * 1024;

    fn sample(pid: Pid, name: &str, private_mb: u64) -> ProcessSample {
        ProcessSample {
            facts: ProcessFacts {
                key: ProcessKey { pid, created: 1 },
                name: name.into(),
                services: vec![ServiceFacts {
                    name: "Audiosrv".into(),
                    accepts_stop: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            metrics: ProcessMetrics {
                private_bytes: private_mb * MB,
                memory_bytes: private_mb * MB,
                ..Default::default()
            },
        }
    }

    fn class(role: Role) -> Classification {
        Classification {
            role,
            root_pid: 0,
            in_foreground_tree: false,
            has_visible_window: false,
            tree_has_window: false,
            plays_audio: false,
            emits_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: Some("Audiosrv".into()),
            schedule: ScheduleScope::Excluded,
        }
    }

    fn input<'a>(
        snap: &'a Snapshot,
        classes: &'a HashMap<Pid, Classification>,
        suspects: &'a [LeakSuspect],
        trigger: Trigger,
        immersive: bool,
    ) -> PlanInput<'a> {
        PlanInput {
            snapshot: snap,
            classes,
            suspects,
            grown: &[],
            trigger,
            foreground_immersive: immersive,
            restart_advised: true,
        }
    }

    fn world() -> (Snapshot, HashMap<Pid, Classification>) {
        let snap = Snapshot {
            processes: vec![
                sample(1, "audiodg.exe", 1065),
                sample(2, "dwm.exe", 1200),
                sample(3, "helper.exe", 300),
                sample(4, "wechat.exe", 2000),
            ],
            system: SystemMetrics {
                kernel_nonpaged: 1675 * MB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut windowed = class(Role::UserApp);
        windowed.has_visible_window = true;
        let classes = HashMap::from([
            (
                1,
                class(Role::ServiceChild {
                    service: "Audiosrv".into(),
                }),
            ),
            (2, class(Role::Compositor)),
            (3, class(Role::UserApp)),
            (4, windowed),
        ]);
        (snap, classes)
    }

    fn leak(pid: Pid) -> LeakSuspect {
        LeakSuspect {
            key: ProcessKey { pid, created: 1 },
            metric: LeakMetric::Memory,
            per_hour: 1e8,
        }
    }

    #[test]
    fn manual_cleanup_runs_everything_including_disruptive() {
        let (snap, classes) = world();
        let suspects = [leak(3)];
        let p = plan(
            &CleanupConfig::default(),
            &input(&snap, &classes, &suspects, Trigger::Manual, false),
        );
        let actions: Vec<_> = p.run_now.iter().map(|s| &s.action).collect();
        assert!(actions.contains(&&Action::RestartService {
            pid: 1,
            service: "Audiosrv".into()
        }));
        assert!(actions.contains(&&Action::RecreateCompositor { pid: 2 }));
        assert!(actions.contains(&&Action::RestartApp { pid: 3 }));
        // 有窗口的程序只提示，不动手。
        assert!(!p.run_now.iter().any(|s| s.key.pid == 4));
        assert!(
            p.advisories
                .iter()
                .any(|a| matches!(a, Advisory::HeavyApp { pid: 4, .. }))
        );
        assert!(
            p.advisories
                .iter()
                .any(|a| matches!(a, Advisory::RestartRecommended { .. }))
        );
        // 无感的排在前面。
        assert!(!p.run_now[0].disruptive);
    }

    #[test]
    fn manual_cleanup_defers_disruptive_steps_while_gaming() {
        let (snap, classes) = world();
        let p = plan(
            &CleanupConfig::default(),
            &input(&snap, &classes, &[], Trigger::Manual, true),
        );
        assert!(p.run_now.iter().all(|s| !s.disruptive));
        assert_eq!(p.deferred.len(), 2);
    }

    #[test]
    fn auto_cleanup_only_does_silent_steps() {
        let (snap, classes) = world();
        let suspects = [leak(3)];
        let p = plan(
            &CleanupConfig::default(),
            &input(&snap, &classes, &suspects, Trigger::Auto, false),
        );
        assert_eq!(p.run_now.len(), 1);
        assert_eq!(p.run_now[0].action, Action::RestartApp { pid: 3 });
        assert!(p.advisories.contains(&Advisory::ManualPending { count: 2 }));
    }

    #[test]
    fn healthy_processes_produce_no_steps() {
        let snap = Snapshot {
            processes: vec![sample(1, "audiodg.exe", 20)],
            ..Default::default()
        };
        let classes = HashMap::from([(
            1,
            class(Role::ServiceChild {
                service: "Audiosrv".into(),
            }),
        )]);
        let p = plan(
            &CleanupConfig::default(),
            &input(&snap, &classes, &[], Trigger::Manual, false),
        );
        assert!(p.is_empty());
    }

    #[test]
    fn same_service_is_restarted_once() {
        let snap = Snapshot {
            processes: vec![sample(1, "a.exe", 900), sample(2, "b.exe", 900)],
            ..Default::default()
        };
        let svc = || {
            class(Role::ServiceChild {
                service: "Audiosrv".into(),
            })
        };
        let classes = HashMap::from([(1, svc()), (2, svc())]);
        let p = plan(
            &CleanupConfig::default(),
            &input(&snap, &classes, &[], Trigger::Manual, false),
        );
        assert_eq!(p.run_now.len(), 1);
    }

    #[test]
    fn processes_grown_past_their_baseline_are_cleaned_like_leaks() {
        use crate::cleanup::Change;
        let snap = Snapshot {
            processes: vec![sample(3, "helper.exe", 50)],
            ..Default::default()
        };
        let classes = HashMap::from([(3, class(Role::UserApp))]);
        let grown = [Grown {
            key: ProcessKey { pid: 3, created: 1 },
            change: Change {
                metric: GrowthMetric::Handles,
                from: 1_000,
                to: 40_000,
            },
        }];
        let p = plan(
            &CleanupConfig::default(),
            &PlanInput {
                grown: &grown,
                ..input(&snap, &classes, &[], Trigger::Manual, false)
            },
        );
        assert_eq!(p.run_now.len(), 1);
        assert_eq!(
            p.run_now[0].reason,
            CleanupReason::Grown(GrowthMetric::Handles)
        );
    }
}
