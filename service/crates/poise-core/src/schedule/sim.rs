//! 模拟复盘：按 2 秒一轮推进时间，模拟真实使用场景（玩游戏、来回切换、后台下载、播放声音），
//! 检查调度器"轻"和"稳"——闲着的程序从不被碰、正在用的程序从不被降级、同一个程序不会被来回调整。

use std::collections::HashMap;

use super::*;
use crate::action::{Action, CoreSet, EcoQos};
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
    last_plan: Option<SchedulePlan>,
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
            last_plan: None,
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
        });
        for a in &plan.actions {
            self.scheduler.report(a, &Ok(ExecOutcome::default()));
            self.log.push((self.now, a.clone()));
        }
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

    fn never_lowered(&self, pid: Pid) -> bool {
        self.actions_on(pid).iter().all(|a| match a {
            Action::SetPriority { class, .. } => class.rank() >= PriorityClass::Normal.rank(),
            Action::SetEcoQos { state, .. } => *state != EcoQos::On,
            Action::SetCoreSet { set, .. } => *set == CoreSet::All,
            _ => true,
        })
    }
}

/// 复盘"玩游戏时前台卡、声音卡"的场景：后台什么都没干，只有游戏、放着歌的浏览器和一堆托盘程序。
#[test]
fn replay_gaming_with_an_idle_background() {
    const GAME: Pid = 1;
    const BROWSER: Pid = 2;
    const RENDERER: Pid = 3;
    let mut procs = vec![
        Proc {
            cpu_pct: 350.0,
            renders_fullscreen: true,
            ..app(GAME)
        },
        app(BROWSER),
        Proc {
            cpu_pct: 8.0,
            ..child(RENDERER, BROWSER)
        },
        Proc {
            audio: true,
            ..child(4, BROWSER)
        },
    ];
    procs.extend((10..30).map(app));
    let mut sim = Sim::new(procs);

    sim.foreground = Some(GAME);
    sim.run_for(60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 玩到一半切出去看一眼浏览器，再切回来，来回几次。
    for _ in 0..6 {
        sim.foreground = Some(BROWSER);
        sim.run_for(10_000);
        sim.foreground = Some(GAME);
        sim.run_for(20_000);
    }
    sim.run_for(5 * 60_000);

    assert_eq!(sim.mode(), Mode::Full);
    // 后台什么都没干，调度就什么都不做：没有任何一个进程被调整过。
    assert!(sim.log.is_empty(), "{:?}", sim.log);
}

/// 复盘"不玩游戏，在几个程序之间来回切换"：调度不应该跟着来回折腾。
#[test]
fn replay_switching_between_busy_programs() {
    // 三个程序都在干活（编辑器、浏览器、聊天软件），用户在它们之间来回切换。
    let mut sim = Sim::new(
        [1, 2, 3]
            .map(|pid| Proc {
                cpu_pct: 30.0,
                ..app(pid)
            })
            .to_vec(),
    );
    // 程序是用户打开的：打开时都在前台待过。
    for pid in [1, 2, 3] {
        sim.foreground = Some(pid);
        sim.run_for(TICK_MS);
    }
    for round in 0..60 {
        sim.foreground = Some([1, 2, 3][round % 3]);
        sim.run_for(10_000);
    }
    // 最近用过的程序都算"正在用"，切来切去一次都没被降级。
    assert!(sim.log.is_empty(), "{:?}", sim.log);
}

/// 复盘"后台有个程序在持续干活"：它只在持续忙了一阵后让路一次，之后一直保持；
/// 用户点回它时立即恢复。
#[test]
fn replay_background_work_yields_once() {
    const UPDATER: Pid = 40;
    let mut procs = vec![app(1)];
    procs.extend((10..20).map(app));
    procs.push(app(UPDATER));
    let mut sim = Sim::new(procs);
    sim.foreground = Some(1);
    sim.run_for(30_000);
    assert!(sim.log.is_empty());

    // 短暂的活动（4 秒）不让路。
    sim.proc_mut(UPDATER).cpu_pct = 60.0;
    sim.run_for(4_000);
    sim.proc_mut(UPDATER).cpu_pct = 0.0;
    sim.run_for(10_000);
    assert!(sim.log.is_empty());

    // 持续忙：10 秒后让路。
    sim.proc_mut(UPDATER).cpu_pct = 60.0;
    sim.run_for(20_000);
    assert!(sim.is_yielded(UPDATER));
    // 时忙时闲也不来回调整。
    for _ in 0..10 {
        sim.proc_mut(UPDATER).cpu_pct = 0.0;
        sim.run_for(10_000);
        sim.proc_mut(UPDATER).cpu_pct = 60.0;
        sim.run_for(10_000);
    }
    assert!(sim.is_yielded(UPDATER));
    assert_eq!(sim.adjustments_per_process(), HashMap::from([(UPDATER, 1)]));

    // 用户点开它：立即恢复。
    sim.foreground = Some(UPDATER);
    sim.run_for(TICK_MS);
    assert!(!sim.is_yielded(UPDATER));
    assert!(sim.never_lowered(1));
    for pid in 10..20 {
        assert!(sim.actions_on(pid).is_empty());
    }
}

#[test]
fn programs_playing_audio_are_never_touched_even_when_busy() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            cpu_pct: 80.0,
            ..app(2)
        },
        Proc {
            audio: true,
            ..child(3, 2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(10 * 60_000);
    assert!(sim.log.is_empty(), "{:?}", sim.log);
}

#[test]
fn games_switch_the_mode_and_stay_active_when_alt_tabbed() {
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 300.0,
            renders_fullscreen: true,
            ..app(1)
        },
        app(2),
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    // 自动识别的游戏要持续几秒才确认。
    assert_eq!(sim.mode(), Mode::Balanced);
    sim.run_for(6_000);
    assert_eq!(sim.mode(), Mode::Full);
    assert_eq!(
        sim.last_plan.as_ref().unwrap().game.as_ref().map(|g| g.pid),
        Some(1)
    );
    sim.foreground = Some(2);
    sim.run_for(10 * 60_000);
    assert_eq!(sim.mode(), Mode::Full);
    // 游戏从不被降级，也从不被提速。
    assert!(sim.actions_on(1).is_empty());
}

#[test]
fn low_priority_programs_stay_on_small_cores() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert!(sim.is_yielded(2));
    sim.foreground = Some(2);
    sim.run_for(TICK_MS);
    assert!(matches!(
        sim.scheduler.state(&ProcessKey { pid: 2, created: 1 }),
        ProcessSchedule::Adjusted(d) if d == Desired::LOW_IN_FOREGROUND
    ));
}

#[test]
fn turning_smart_scheduling_off_restores_everything() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert_eq!(sim.scheduler.adjusted_count(), 1);
    sim.tick_with(false);
    assert_eq!(sim.scheduler.adjusted_count(), 0);
}

#[test]
fn non_hybrid_cpus_never_restrict_cores() {
    let mut sim = Sim::with(
        ScheduleConfig {
            hybrid: false,
            ..Default::default()
        },
        vec![
            app(1),
            Proc {
                preference: AppPreference::Background,
                ..app(2)
            },
        ],
    );
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    assert!(!sim.actions_on(2).is_empty());
    assert!(
        !sim.log
            .iter()
            .any(|(_, a)| matches!(a.action, Action::SetCoreSet { .. }))
    );
}

#[test]
fn release_all_restores_every_adjusted_process() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    let restore = sim.scheduler.release_all();
    assert!(!restore.is_empty());
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

#[test]
fn children_that_inherited_low_priority_are_restored_to_normal() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            preference: AppPreference::Background,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(TICK_MS);
    // 父进程已被降低；之后新建的子进程继承了"低于正常"（Windows 的规则）。
    sim.procs.push(child(3, 2));
    let mut snap = sim.snapshot();
    for p in &mut snap.processes {
        if p.facts.key.pid == 3 {
            p.facts.priority = Some(PriorityClass::BelowNormal);
            p.facts.key.created = 2;
        }
    }
    sim.now += TICK_MS;
    snap.at = sim.now;
    let classes = sim.classes();
    let plan = sim.scheduler.plan(&ScheduleInput {
        snapshot: &snap,
        classes: &classes,
        activity: &sim.activity,
        enabled: true,
        user_mode: Mode::Balanced,
        game_mode: None,
    });
    for a in &plan.actions {
        sim.scheduler.report(a, &Ok(ExecOutcome::default()));
    }
    // 子进程自己不需要让路：立即恢复为正常，不会一直低着。
    assert!(plan.actions.iter().any(|a| a.key.pid == 3
        && a.action
            == Action::SetPriority {
                pid: 3,
                class: PriorityClass::Normal
            }));
}
