//! 进程调度：每个周期按调度计划调整进程，临时全速，服务停止时撤销全部调整；程序偏好页的调度状态。

use std::collections::HashMap;

use poise_core::action::{CoreSet, ExecError, ExecOutcome};
use poise_core::classify::Classification;
use poise_core::model::{Pid, ProcessKey, ProcessSample, Snapshot};
use poise_core::prefs::Mode;
use poise_core::safety::{GuardedExecutor, Trigger};
use poise_core::schedule::{Desired, GameInfo, PlannedAction, ProcessSchedule, ScheduleInput};
use poise_protocol::{AppSchedule, ErrorCode, ScheduleState};

use super::error::RequestError;
use super::{Engine, convert};

/// 临时全速的最长时间（分钟）。
const MAX_BOOST_MINUTES: u32 = 240;

pub(super) struct ScheduleOutcome {
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
}

impl Engine {
    /// 起作用时调度；暂停时什么都不做，模式按用户设置显示（不落到硬件）。
    pub(super) fn schedule_if(
        &mut self,
        active: bool,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
    ) -> ScheduleOutcome {
        if active {
            self.schedule(snapshot, classes)
        } else {
            ScheduleOutcome {
                effective_mode: self.settings.mode,
                game: None,
            }
        }
    }

    /// 生成调度计划并执行其中的进程调整，返回实际模式和当前游戏。
    fn schedule(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
    ) -> ScheduleOutcome {
        let now = self.now();
        let user_mode = self.resolve_user_mode(snapshot, classes, now);
        let plan = self.scheduler.plan(&ScheduleInput {
            snapshot,
            classes,
            activity: &self.activity,
            enabled: self.settings.smart_schedule,
            user_mode,
            game_mode: self.game_target(),
        });
        self.execute_plan(&plan.actions, classes);
        ScheduleOutcome {
            effective_mode: plan.effective_mode,
            game: plan.game,
        }
    }

    /// 执行调度动作（经过安全守卫），并把结果回报给调度器。
    fn execute_plan(&mut self, actions: &[PlannedAction], classes: &HashMap<Pid, Classification>) {
        for planned in actions {
            let Some(result) = execute(&self.executor, planned, classes) else {
                continue;
            };
            match &result {
                Ok(_) if self.cfg.dry_run => log::info!("[观察] {:?}", planned.action),
                Ok(_) => log::debug!("{:?}", planned.action),
                Err(e) => log::debug!("{:?} 未执行：{e}", planned.action),
            }
            self.scheduler.report(planned, &result);
        }
    }

    /// 撤销所有进程调整（服务停止、Poise 退出时）。按最新的进程快照撤销两遍：
    /// 第二遍兜住撤销期间新建、从父进程继承了"低于正常"的子进程。没能撤销的调整仍记着，之后再撤销。
    pub(super) fn release_processes(&mut self) {
        for _ in 0..2 {
            let now = self.now();
            let Ok(snapshot) = self.snapshotter.take(now) else {
                return;
            };
            let classes = self.classify(&snapshot, now);
            let plan = self.scheduler.plan(&ScheduleInput {
                snapshot: &snapshot,
                classes: &classes,
                activity: &self.activity,
                enabled: false,
                user_mode: self.settings.mode,
                game_mode: None,
            });
            self.execute_plan(&plan.actions, &classes);
        }
    }

    /// 临时全速：在指定时间内按"始终优先"处理该程序。
    pub(super) fn temp_boost(&mut self, pid: Pid, minutes: u32) -> Result<(), RequestError> {
        let key = self
            .process_key(pid)
            .ok_or_else(|| RequestError::new(ErrorCode::ProcessGone, "程序已经退出"))?;
        let until = self.now() + u64::from(minutes.min(MAX_BOOST_MINUTES)) * 60_000;
        self.scheduler.boost(key, until);
        self.reschedule_now();
        Ok(())
    }

    /// 一个程序（进程树的主进程）当前的调度状态。
    pub(super) fn app_schedule(&self, p: &ProcessSample, c: &Classification) -> AppSchedule {
        let schedule = self.scheduler.state(&p.facts.key);
        let state = if !self.settings.smart_schedule {
            ScheduleState::Off
        } else if !c.is_schedulable() {
            ScheduleState::Excluded
        } else {
            match schedule {
                ProcessSchedule::SelfManaged => ScheduleState::SelfManaged,
                ProcessSchedule::Denied => ScheduleState::Excluded,
                ProcessSchedule::Default | ProcessSchedule::Adjusted(_) => ScheduleState::Scheduled,
            }
        };
        let applied = match schedule {
            ProcessSchedule::Adjusted(d) => d,
            _ => Desired::NEUTRAL,
        };
        AppSchedule {
            state,
            foreground: c.in_foreground_tree,
            small_cores: self
                .topology
                .is_hybrid()
                .then_some(applied.cores == CoreSet::Efficiency),
            priority: convert::priority_level(
                applied.priority.or(p.facts.priority).unwrap_or_default(),
            ),
        }
    }

    fn process_key(&self, pid: Pid) -> Option<ProcessKey> {
        self.view.as_ref()?.snapshot.find(pid).map(|p| p.facts.key)
    }
}

/// 经安全执行器执行一个调度动作。进程已经没有分类（刚退出）时不执行，返回 None。
fn execute(
    executor: &GuardedExecutor,
    planned: &PlannedAction,
    classes: &HashMap<Pid, Classification>,
) -> Option<Result<ExecOutcome, ExecError>> {
    let class = classes.get(&planned.key.pid)?;
    Some(executor.execute(
        &planned.action,
        class,
        planned.original_priority,
        Trigger::Auto,
    ))
}
