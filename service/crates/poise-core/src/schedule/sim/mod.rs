//! 模拟复盘：按 2 秒一轮推进时间，模拟真实使用场景（玩游戏、来回切换、后台下载、播放声音），
//! 检查调度器"轻"和"稳"——闲着的程序从不被碰、正在用的程序从不被降级、同一个程序不会被来回调整；
//! 正在玩、正在发声的程序只被保护一次（不让系统自动节能限速），之后不再动。

use std::collections::HashMap;

mod audio;
mod background;
mod games;
mod restore;

use super::*;
use crate::action::{Action, CoreSet, EcoQos, ResourcePriority};
use crate::classify::{Role, ScheduleScope};
use crate::model::{PriorityClass, ProcessFacts, ProcessMetrics};
use crate::prefs::AppPreference;

const TICK_MS: Millis = 2_000;

/// 模拟里的一个进程。
#[derive(Clone)]
struct Proc {
    pid: Pid,
    parent: Option<Pid>,
    /// 所属程序（进程树的根）。
    root: Pid,
    /// 单核占用百分比。
    cpu_pct: f32,
    audio: bool,
    preference: AppPreference,
    /// 全屏渲染时会被识别为游戏。
    renders_fullscreen: bool,
}

fn app(pid: Pid) -> Proc {
    Proc {
        pid,
        parent: None,
        root: pid,
        cpu_pct: 0.2,
        audio: false,
        preference: AppPreference::Auto,
        renders_fullscreen: false,
    }
}

fn child(pid: Pid, parent: Pid) -> Proc {
    Proc {
        parent: Some(parent),
        root: parent,
        ..app(pid)
    }
}

struct Sim {
    scheduler: Scheduler,
    activity: ActivityTracker,
    procs: Vec<Proc>,
    cpu_time: HashMap<Pid, u64>,
    now: Millis,
    /// 前台程序（进程树的根）。
    foreground: Option<Pid>,
    /// 执行过的全部动作（时间、动作）。
    log: Vec<(Millis, PlannedAction)>,
    /// 报告过的全部决定变化。
    decisions: Vec<Decision>,
    last_plan: Option<SchedulePlan>,
    on_battery: bool,
}

impl Sim {
    fn new(procs: Vec<Proc>) -> Self {
        Self::with(ScheduleConfig::default(), procs)
    }

    fn with(cfg: ScheduleConfig, procs: Vec<Proc>) -> Self {
        Self {
            scheduler: Scheduler::new(cfg),
            activity: ActivityTracker::default(),
            procs,
            cpu_time: HashMap::new(),
            now: 0,
            foreground: None,
            log: Vec::new(),
            decisions: Vec::new(),
            last_plan: None,
            on_battery: false,
        }
    }

    fn proc_mut(&mut self, pid: Pid) -> &mut Proc {
        self.procs.iter_mut().find(|p| p.pid == pid).unwrap()
    }

    /// 采样里的优先级反映 Poise 已经设置的值（与真实系统一致）。
    fn snapshot(&mut self) -> Snapshot {
        let processes = self
            .procs
            .iter()
            .map(|p| {
                let cpu = self.cpu_time.entry(p.pid).or_default();
                *cpu += (p.cpu_pct as f64 / 100.0 * TICK_MS as f64 * 1e4) as u64;
                let key = ProcessKey {
                    pid: p.pid,
                    created: 1,
                };
                let priority = match self.scheduler.state(&key) {
                    ProcessSchedule::Adjusted(d) => d.priority.unwrap_or(PriorityClass::Normal),
                    _ => PriorityClass::Normal,
                };
                ProcessSample {
                    facts: ProcessFacts {
                        key,
                        parent_pid: p.parent,
                        name: format!("p{}.exe", p.pid),
                        session_id: 1,
                        priority: Some(priority),
                        ..Default::default()
                    },
                    metrics: ProcessMetrics {
                        cpu_time: *cpu,
                        ..Default::default()
                    },
                }
            })
            .collect();
        Snapshot {
            at: self.now,
            processes,
            ..Default::default()
        }
    }

    fn classes(&self) -> HashMap<Pid, Classification> {
        self.procs
            .iter()
            .map(|p| {
                let foreground = self.foreground == Some(p.root);
                let tree_audio = self.procs.iter().any(|q| q.root == p.root && q.audio);
                let class = Classification {
                    role: Role::UserApp,
                    root_pid: p.root,
                    in_foreground_tree: foreground,
                    has_visible_window: p.parent.is_none(),
                    tree_has_window: true,
                    plays_audio: tree_audio,
                    emits_audio: p.audio,
                    recently_active: p.cpu_pct > 2.0,
                    preference: p.preference,
                    is_game: p.preference == AppPreference::Game
                        || (foreground && p.renders_fullscreen),
                    anti_cheat_present: false,
                    restartable_service: None,
                    schedule: ScheduleScope::App,
                };
                (p.pid, class)
            })
            .collect()
    }

    fn tick_with(&mut self, enabled: bool) {
        self.now += TICK_MS;
        let snap = self.snapshot();
        self.activity.update(&snap);
        let classes = self.classes();
        let plan = self.scheduler.plan(&ScheduleInput {
            snapshot: &snap,
            classes: &classes,
            activity: &self.activity,
            enabled,
            user_mode: Mode::Balanced,
            game_mode: Some(Mode::Full),
            on_battery: self.on_battery,
            hot: false,
        });
        for a in &plan.actions {
            self.scheduler.report(a, &Ok(ExecOutcome::default()));
            self.log.push((self.now, a.clone()));
        }
        self.decisions.extend(plan.decisions.iter().cloned());
        self.last_plan = Some(plan);
    }

    fn run_for(&mut self, ms: Millis) {
        let until = self.now + ms;
        while self.now < until {
            self.tick_with(true);
        }
    }

    fn mode(&self) -> Mode {
        self.last_plan.as_ref().unwrap().effective_mode
    }

    fn actions_on(&self, pid: Pid) -> Vec<&Action> {
        self.log
            .iter()
            .filter(|(_, a)| a.key.pid == pid)
            .map(|(_, a)| &a.action)
            .collect()
    }

    /// 对这个程序先后报告过的决定。
    fn verdicts_of(&self, pid: Pid) -> Vec<Verdict> {
        self.decisions
            .iter()
            .filter_map(|d| match d {
                Decision::Program { root, to, .. } if root.pid == pid => Some(*to),
                _ => None,
            })
            .collect()
    }

    /// 每个进程被调整过的轮数（同一轮里的几个动作算一次）。
    fn adjustments_per_process(&self) -> HashMap<Pid, usize> {
        let mut rounds: HashMap<Pid, Vec<Millis>> = HashMap::new();
        for (at, a) in &self.log {
            let r = rounds.entry(a.key.pid).or_default();
            if r.last() != Some(at) {
                r.push(*at);
            }
        }
        rounds.into_iter().map(|(pid, r)| (pid, r.len())).collect()
    }

    fn is_yielded(&self, pid: Pid) -> bool {
        matches!(
            self.scheduler.state(&ProcessKey { pid, created: 1 }),
            ProcessSchedule::Adjusted(d) if d == Desired::YIELDED
        )
    }

    fn is_protected(&self, pid: Pid) -> bool {
        matches!(
            self.scheduler.state(&ProcessKey { pid, created: 1 }),
            ProcessSchedule::Adjusted(d) if d == Desired::PROTECTED
        )
    }

    /// 对这个进程只做过"不让系统节能限速"，没有改优先级、没有限核心。
    fn only_protected(&self, pid: Pid) -> bool {
        self.actions_on(pid).iter().all(|a| {
            matches!(
                a,
                Action::SetEcoQos {
                    state: EcoQos::Off,
                    ..
                }
            )
        })
    }

    fn state_of(&self, pid: Pid) -> Option<Desired> {
        match self.scheduler.state(&ProcessKey { pid, created: 1 }) {
            ProcessSchedule::Adjusted(d) => Some(d),
            _ => None,
        }
    }

    fn never_lowered(&self, pid: Pid) -> bool {
        self.actions_on(pid).iter().all(|a| match a {
            Action::SetPriority { class, .. } => class.rank() >= PriorityClass::Normal.rank(),
            Action::SetEcoQos { state, .. } => *state != EcoQos::On,
            Action::SetCoreSet { set, .. } => *set != CoreSet::Efficiency,
            Action::SetResourcePriority { priority, .. } => *priority != ResourcePriority::Low,
            _ => true,
        })
    }
}
