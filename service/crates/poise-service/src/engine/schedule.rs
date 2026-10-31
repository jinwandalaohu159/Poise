//! 进程调度：每个周期按调度计划调整进程并记录决定的变化，临时全速，服务停止时撤销全部调整。

use poise_core::action::{ExecError, ExecOutcome};
use poise_core::classify::Classification;
use poise_core::fast_hash::HashMap;
use poise_core::model::{Pid, ProcessKey, Snapshot};
use poise_core::prefs::Mode;
use poise_core::safety::{GuardedExecutor, Trigger};
use poise_core::schedule::{GameInfo, PlannedAction, ScheduleInput};
use poise_protocol::{ErrorCode, Event};

use super::error::RequestError;
use super::{Engine, decision_events};

/// 临时全速的最长时间（分钟）。
const MAX_BOOST_MINUTES: u32 = 240;

pub(super) struct ScheduleOutcome {
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
    /// 有重点程序（见 `poise_core::schedule::SchedulePlan::focused`）。
    pub focused: bool,
}

impl Engine {
    /// 起作用时调度；暂停时什么都不做，模式按用户设置显示（不落到硬件）。
    pub(super) fn schedule_if(
        &mut self,
        active: bool,
        snapshot: &Snapshot,
        previous: Option<&Snapshot>,
        classes: &HashMap<Pid, Classification>,
    ) -> ScheduleOutcome {
        if active {
            self.schedule(snapshot, previous, classes)
        } else {
            ScheduleOutcome {
                effective_mode: self.settings.mode,
                game: None,
                focused: false,
            }
        }
    }

    /// 生成调度计划并执行其中的进程调整，返回实际模式和当前游戏。`previous` 是上一次的采样
    /// （日志里给刚退出的程序取名字）。
    fn schedule(
        &mut self,
        snapshot: &Snapshot,
        previous: Option<&Snapshot>,
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
            on_battery: snapshot.system.on_battery,
            hot: self.headroom.exhausted(),
            user_idle_ms: self.session.current(now).idle_ms,
        });
        self.execute_plan(&plan.actions, snapshot, classes);
        if !plan.actions.is_empty() || !plan.decisions.is_empty() {
            self.last_change_at = now;
        }
        self.adjusted.update(self.scheduler.adjusted(), now);
        let domains = self.topology.domains.len();
        for event in decision_events::events(&plan.decisions, snapshot, previous, domains) {
            self.logbook.event(event);
        }
        ScheduleOutcome {
            effective_mode: plan.effective_mode,
            game: plan.game,
            focused: plan.focused,
        }
    }

    /// 执行调度动作（经过安全守卫），并把结果回报给调度器。
    fn execute_plan(
        &mut self,
        actions: &[PlannedAction],
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
    ) {
        for planned in actions {
            let Some(result) = execute(&self.executor, planned, classes) else {
                continue;
            };
            match &result {
                Ok(_) if self.cfg.dry_run => log::info!("[观察] {:?}", planned.action),
                Ok(_) => log::debug!("{:?}", planned.action),
                // 系统拒绝访问的进程之后不再尝试（见 `Reconciler::report`），每个进程只记一次。
                // 游戏被拒绝时，它就没能放到大核上：日志里要看得到。
                Err(ExecError::AccessDenied) => {
                    self.logbook.event_with(
                        Event::AccessDenied {
                            program: decision_events::program_ref(planned.key, [snapshot]),
                        },
                        format!("{:?}", planned.action),
                    );
                }
                Err(e) => log::debug!("{:?} 未执行：{e}", planned.action),
            }
            self.scheduler.report(planned, &result);
        }
    }

    /// 上次运行异常退出、没能撤销的调整：把还在运行的那些进程恢复原样（启动后第一轮做一次，
    /// 不管调度是否起作用）。随后的调度会按当前情况重新决定它们该怎样。
    pub(super) fn restore_leftovers(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
    ) {
        let Some(leftovers) = self.adjusted.take_leftovers() else {
            return;
        };
        let actions = self.scheduler.restore_leftovers(&leftovers, snapshot);
        let restored = actions
            .iter()
            .filter(|a| execute(&self.executor, a, classes).is_some_and(|r| r.is_ok()))
            .map(|a| a.key)
            .collect::<std::collections::HashSet<_>>()
            .len();
        self.logbook.event(Event::Recovered {
            left: leftovers.len() as u32,
            restored: restored as u32,
        });
    }

    /// 撤销所有进程调整（服务停止、Poise 退出时）。按最新的进程快照撤销两遍：
    /// 第二遍兜住撤销期间新建、从父进程继承了"低于正常"的子进程。没能撤销的调整仍记着，之后再撤销。
    pub(super) fn release_processes(&mut self) {
        for _ in 0..2 {
            let now = self.now();
            let Some(snapshot) = self.sampler.take(now) else {
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
                on_battery: false,
                hot: false,
                user_idle_ms: 0,
            });
            self.execute_plan(&plan.actions, &snapshot, &classes);
        }
        let now = self.now();
        self.adjusted.flush(self.scheduler.adjusted(), now);
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
        planned.key,
        &planned.action,
        class,
        planned.original_priority,
        Trigger::Auto,
    ))
}
