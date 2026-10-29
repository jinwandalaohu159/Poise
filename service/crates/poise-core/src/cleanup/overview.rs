//! 清理页的明细：像任务管理器一样按程序列出占用，并标出每一项"怎么了、会怎么处理"。
//! 同一程序树的进程合并为一行；需要处理的排在前面，其余按内存占用从高到低。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::growth::{Change, Grown};
use super::leak::{LeakMetric, LeakSuspect};
use super::planner::{Advisory, CleanupPlan, CleanupReason};
use crate::action::Action;
use crate::classify::{Classification, Role};
use crate::model::{Pid, ProcessSample, Snapshot};

/// 这一项的状况。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    None,
    /// 内存持续增长（按趋势判断）。
    Growing,
    /// 句柄、线程或内存比进程稳定运行时多出数倍，且一直没有回落。
    Accumulated,
    /// 占用远超正常水平（例如音频引擎超过数百 MB）。
    Oversized,
    /// 有窗口的程序占用很多：建议用户自己重新打开。
    Heavy,
}

/// 一键清理会怎样处理这一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Remedy {
    None,
    /// 重启该服务（进程由服务重新拉起）。
    RestartService {
        service: String,
    },
    /// 系统自动重建桌面合成器（屏幕闪一下）。
    RecreateCompositor,
    /// 后台程序：结束后按原命令行以用户身份重新启动。
    RestartApp,
    /// 只提示，由用户自己重新打开。
    ReopenManually,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessGroup {
    /// 主进程（用于取图标和定位）。
    pub pid: Pid,
    pub name: String,
    pub path: Option<String>,
    pub process_count: usize,
    /// 实际占用的物理内存（专用工作集，与任务管理器一致）。
    pub memory_bytes: u64,
    /// 占整机 CPU 的百分比（与任务管理器一致）。
    pub cpu_pct: f32,
    pub issue: Issue,
    /// 内存增长速度（字节/小时），仅在 [`Issue::Growing`] 时有值。
    pub growth_per_hour: Option<u64>,
    /// 与进程稳定运行时相比的变化，仅在 [`Issue::Accumulated`] 时有值。
    pub since_start: Option<Change>,
    pub remedy: Remedy,
    /// 处理时会造成短暂打扰（断音、闪屏）。
    pub disruptive: bool,
}

impl ProcessGroup {
    pub fn needs_attention(&self) -> bool {
        self.issue != Issue::None
    }
}

pub struct OverviewInput<'a> {
    pub snapshot: &'a Snapshot,
    pub classes: &'a HashMap<Pid, Classification>,
    /// 按"手动清理"生成的计划（与点按钮时执行的一致）。
    pub plan: &'a CleanupPlan,
    pub suspects: &'a [LeakSuspect],
    pub grown: &'a [Grown],
    /// 每个进程占整机 CPU 的百分比。
    pub cpu_pct: &'a HashMap<Pid, f32>,
}

/// 不需要处理的程序最多列出多少行。
pub const MAX_NORMAL_ROWS: usize = 20;

pub fn overview(input: &OverviewInput) -> Vec<ProcessGroup> {
    let by_pid = input.snapshot.by_pid();
    let mut groups: HashMap<Pid, ProcessGroup> = HashMap::new();
    for p in &input.snapshot.processes {
        let pid = p.facts.key.pid;
        let Some(class) = input.classes.get(&pid) else {
            continue;
        };
        if matches!(class.role, Role::Critical | Role::SelfProcess) {
            continue;
        }
        let head = group_head(p, class, &by_pid, input.classes);
        let g = groups
            .entry(head.facts.key.pid)
            .or_insert_with(|| ProcessGroup {
                pid: head.facts.key.pid,
                name: head.facts.friendly_name(),
                path: head.facts.image_path.clone(),
                process_count: 0,
                memory_bytes: 0,
                cpu_pct: 0.0,
                issue: Issue::None,
                growth_per_hour: None,
                since_start: None,
                remedy: Remedy::None,
                disruptive: false,
            });
        g.process_count += 1;
        g.memory_bytes += p.metrics.memory_bytes;
        g.cpu_pct += input.cpu_pct.get(&pid).copied().unwrap_or(0.0);
        if let Some(finding) = finding_for(p, input) {
            apply_finding(g, finding);
        }
    }

    let (mut flagged, mut normal): (Vec<_>, Vec<_>) = groups
        .into_values()
        .partition(ProcessGroup::needs_attention);
    flagged.sort_by_key(|g| std::cmp::Reverse(g.memory_bytes));
    normal.sort_by_key(|g| std::cmp::Reverse(g.memory_bytes));
    normal.truncate(MAX_NORMAL_ROWS);
    flagged.extend(normal);
    flagged
}

/// 普通程序归到所在进程树的主程序；系统进程各自成行。
fn group_head<'a>(
    p: &'a ProcessSample,
    class: &Classification,
    by_pid: &HashMap<Pid, &'a ProcessSample>,
    classes: &HashMap<Pid, Classification>,
) -> &'a ProcessSample {
    if class.role != Role::UserApp {
        return p;
    }
    by_pid
        .get(&class.root_pid)
        .copied()
        .filter(|root| {
            classes
                .get(&root.facts.key.pid)
                .is_some_and(|c| c.role == Role::UserApp)
        })
        .unwrap_or(p)
}

struct Finding {
    issue: Issue,
    growth_per_hour: Option<u64>,
    since_start: Option<Change>,
    remedy: Remedy,
    disruptive: bool,
}

/// 关于这个进程的结论。相对基线异常增长的进程即使没有安全的处理办法也列出来（处理方式为空）。
fn finding_for(p: &ProcessSample, input: &OverviewInput) -> Option<Finding> {
    let since_start = input
        .grown
        .iter()
        .find(|g| g.key == p.facts.key)
        .map(|g| g.change);
    let planned = planned_finding(p, input);
    match (planned, since_start) {
        (Some(mut f), Some(change)) if f.issue != Issue::Growing => {
            f.issue = Issue::Accumulated;
            f.since_start = Some(change);
            Some(f)
        }
        (None, Some(change)) => Some(Finding {
            issue: Issue::Accumulated,
            growth_per_hour: None,
            since_start: Some(change),
            remedy: Remedy::None,
            disruptive: false,
        }),
        (planned, _) => planned,
    }
}

/// 计划里关于这个进程的结论（清理步骤或"建议重新打开"）。
fn planned_finding(p: &ProcessSample, input: &OverviewInput) -> Option<Finding> {
    let plan = input.plan;
    if let Some(step) = plan
        .run_now
        .iter()
        .chain(&plan.deferred)
        .find(|s| s.key == p.facts.key)
    {
        let growth = input
            .suspects
            .iter()
            .find(|s| s.key == p.facts.key && s.metric == LeakMetric::Memory)
            .map(|s| s.per_hour.max(0.0) as u64);
        return Some(Finding {
            issue: match step.reason {
                CleanupReason::Leak(_) => Issue::Growing,
                CleanupReason::Grown(_) => Issue::Accumulated,
                CleanupReason::Oversized => Issue::Oversized,
            },
            growth_per_hour: growth,
            since_start: None,
            remedy: remedy_of(&step.action),
            disruptive: step.disruptive,
        });
    }
    plan.advisories
        .iter()
        .any(|a| matches!(a, Advisory::HeavyApp { pid, .. } if *pid == p.facts.key.pid))
        .then_some(Finding {
            issue: Issue::Heavy,
            growth_per_hour: None,
            since_start: None,
            remedy: Remedy::ReopenManually,
            disruptive: false,
        })
}

fn remedy_of(action: &Action) -> Remedy {
    match action {
        Action::RestartService { service, .. } => Remedy::RestartService {
            service: service.clone(),
        },
        Action::RecreateCompositor { .. } => Remedy::RecreateCompositor,
        Action::RestartApp { .. } => Remedy::RestartApp,
        _ => Remedy::None,
    }
}

/// 一组里有多个结论时，保留最需要关注的：持续增长 > 比启动时多出数倍 > 占用过高 > 建议重新打开。
fn apply_finding(g: &mut ProcessGroup, f: Finding) {
    let rank = |i: Issue| match i {
        Issue::Growing => 4,
        Issue::Accumulated => 3,
        Issue::Oversized => 2,
        Issue::Heavy => 1,
        Issue::None => 0,
    };
    if rank(f.issue) > rank(g.issue) {
        g.issue = f.issue;
        g.growth_per_hour = f.growth_per_hour;
        g.since_start = f.since_start;
        g.remedy = f.remedy;
        g.disruptive = f.disruptive;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::ScheduleScope;
    use crate::cleanup::CleanupStep;
    use crate::model::{ProcessFacts, ProcessKey, ProcessMetrics};
    use crate::prefs::AppPreference;

    const MB: u64 = 1024 * 1024;

    fn sample(pid: Pid, name: &str, mb: u64) -> ProcessSample {
        ProcessSample {
            facts: ProcessFacts {
                key: ProcessKey { pid, created: 1 },
                name: name.into(),
                ..Default::default()
            },
            metrics: ProcessMetrics {
                private_bytes: mb * MB,
                memory_bytes: mb * MB,
                ..Default::default()
            },
        }
    }

    fn class(role: Role, root_pid: Pid) -> Classification {
        Classification {
            role,
            root_pid,
            in_foreground_tree: false,
            has_visible_window: false,
            tree_has_window: false,
            plays_audio: false,
            emits_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: None,
            schedule: ScheduleScope::App,
        }
    }

    fn step(pid: Pid, reason: CleanupReason, action: Action, disruptive: bool) -> CleanupStep {
        CleanupStep {
            key: ProcessKey { pid, created: 1 },
            action,
            disruptive,
            reason,
            name: String::new(),
            memory_bytes: 0,
        }
    }

    struct World {
        snapshot: Snapshot,
        classes: HashMap<Pid, Classification>,
        plan: CleanupPlan,
        suspects: Vec<LeakSuspect>,
        grown: Vec<Grown>,
        cpu: HashMap<Pid, f32>,
    }

    impl World {
        fn run(&self) -> Vec<ProcessGroup> {
            overview(&OverviewInput {
                snapshot: &self.snapshot,
                classes: &self.classes,
                plan: &self.plan,
                suspects: &self.suspects,
                grown: &self.grown,
                cpu_pct: &self.cpu,
            })
        }
    }

    fn world() -> World {
        World {
            snapshot: Snapshot {
                processes: vec![
                    sample(1, "audiodg.exe", 1065),
                    sample(10, "chrome.exe", 300),
                    sample(11, "chrome.exe", 500),
                    sample(12, "chrome.exe", 200),
                    sample(20, "wechat.exe", 2000),
                    sample(30, "memcompression", 900),
                    sample(40, "poise-agent.exe", 2),
                ],
                ..Default::default()
            },
            classes: HashMap::from([
                (
                    1,
                    class(
                        Role::ServiceChild {
                            service: "Audiosrv".into(),
                        },
                        1,
                    ),
                ),
                (10, class(Role::UserApp, 10)),
                (11, class(Role::UserApp, 10)),
                (12, class(Role::UserApp, 10)),
                (20, class(Role::UserApp, 20)),
                (30, class(Role::Critical, 30)),
                (40, class(Role::SelfProcess, 40)),
            ]),
            plan: CleanupPlan {
                run_now: vec![step(
                    1,
                    CleanupReason::Oversized,
                    Action::RestartService {
                        pid: 1,
                        service: "Audiosrv".into(),
                    },
                    true,
                )],
                deferred: vec![],
                advisories: vec![Advisory::HeavyApp {
                    pid: 20,
                    name: "微信".into(),
                    memory_bytes: 2000 * MB,
                }],
            },
            suspects: vec![],
            grown: vec![],
            cpu: HashMap::from([(10, 1.0), (11, 2.5), (1, 3.0)]),
        }
    }

    #[test]
    fn processes_of_one_app_are_one_row() {
        let rows = world().run();
        let chrome = rows.iter().find(|r| r.pid == 10).unwrap();
        assert_eq!(chrome.process_count, 3);
        assert_eq!(chrome.memory_bytes, 1000 * MB);
        assert!((chrome.cpu_pct - 3.5).abs() < 1e-6);
        assert_eq!(chrome.issue, Issue::None);
    }

    #[test]
    fn flagged_rows_come_first_with_their_remedy() {
        let rows = world().run();
        // 微信 2000 MB 排在音频引擎 1065 MB 前面；两者都在 Chrome 之前。
        assert_eq!(
            rows.iter().map(|r| r.pid).collect::<Vec<_>>(),
            vec![20, 1, 10]
        );
        assert_eq!(
            (rows[0].issue, &rows[0].remedy),
            (Issue::Heavy, &Remedy::ReopenManually)
        );
        assert_eq!(rows[1].issue, Issue::Oversized);
        assert_eq!(
            rows[1].remedy,
            Remedy::RestartService {
                service: "Audiosrv".into()
            }
        );
        assert!(rows[1].disruptive);
    }

    #[test]
    fn critical_and_own_processes_are_not_listed() {
        let rows = world().run();
        assert!(rows.iter().all(|r| r.pid != 30 && r.pid != 40));
    }

    #[test]
    fn growth_speed_is_reported_for_leaks() {
        let mut w = world();
        w.plan.run_now.push(step(
            12,
            CleanupReason::Leak(LeakMetric::Memory),
            Action::RestartApp { pid: 12 },
            false,
        ));
        w.suspects.push(LeakSuspect {
            key: ProcessKey {
                pid: 12,
                created: 1,
            },
            metric: LeakMetric::Memory,
            per_hour: 80.0 * MB as f64,
        });
        let rows = w.run();
        let chrome = rows.iter().find(|r| r.pid == 10).unwrap();
        assert_eq!(chrome.issue, Issue::Growing);
        assert_eq!(chrome.growth_per_hour, Some(80 * MB));
        assert_eq!(chrome.remedy, Remedy::RestartApp);
    }

    #[test]
    fn normal_rows_are_capped() {
        let mut w = world();
        for pid in 100..140 {
            w.snapshot.processes.push(sample(pid, "x.exe", 10));
            w.classes.insert(pid, class(Role::SystemOther, pid));
        }
        let rows = w.run();
        assert_eq!(
            rows.iter().filter(|r| !r.needs_attention()).count(),
            MAX_NORMAL_ROWS
        );
    }

    #[test]
    fn processes_grown_past_their_baseline_are_listed_even_without_a_remedy() {
        use crate::cleanup::GrowthMetric;
        let mut w = world();
        w.snapshot.processes.push(sample(50, "svchost.exe", 80));
        w.classes.insert(50, class(Role::SystemOther, 50));
        let change = Change {
            metric: GrowthMetric::Handles,
            from: 2_000,
            to: 45_000,
        };
        w.grown.push(Grown {
            key: ProcessKey {
                pid: 50,
                created: 1,
            },
            change,
        });
        // 有窗口的程序：仍然只建议重新打开，但原因是"比启动时多"。
        w.grown.push(Grown {
            key: ProcessKey {
                pid: 20,
                created: 1,
            },
            change,
        });
        let rows = w.run();
        let svchost = rows.iter().find(|r| r.pid == 50).unwrap();
        assert_eq!(
            (svchost.issue, &svchost.remedy, svchost.since_start),
            (Issue::Accumulated, &Remedy::None, Some(change))
        );
        let wechat = rows.iter().find(|r| r.pid == 20).unwrap();
        assert_eq!(
            (wechat.issue, &wechat.remedy),
            (Issue::Accumulated, &Remedy::ReopenManually)
        );
    }
}
