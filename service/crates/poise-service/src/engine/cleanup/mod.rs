//! 一键清理、延后清理、自动清理，以及最近一次清理的记录。这里生成计划、记录结果（在引擎锁内，很快）；
//! 执行在锁外（见 [`job`]）。

mod job;

use poise_core::classify::Classification;
use poise_core::cleanup::{
    self, CleanupConfig, CleanupPlan, CleanupStep, Grown, LeakSuspect, OverviewInput, PlanInput,
};
use poise_core::fast_hash::HashMap;
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::safety::Trigger;
use poise_protocol::{
    CleanupOverview, CleanupResult, CleanupTrigger, ErrorCode, Event, LastCleanup,
};

use super::error::RequestError;
use super::{Engine, convert, unix_now_ms};
use job::{Background, ManualSlot, ReadyStep};
pub use job::{CleanupDone, CleanupJob, ManualTurn};

/// 清理的状态。
#[derive(Default)]
pub(super) struct CleanupState {
    deferred: Vec<CleanupStep>,
    last_auto_at: Millis,
    suspects: Vec<LeakSuspect>,
    /// 与自身基线相比异常增长的进程，与泄漏嫌疑同时刷新。
    grown: Vec<Grown>,
    suspects_at: Option<Millis>,
    /// 健康状态用的清理预演与时间（见 [`Engine::cleanup_preview`]）。
    preview: Option<(Millis, CleanupPlan)>,
    last: Option<LastCleanup>,
    /// 正在单独的线程里执行的自动清理或延后的清理。
    background: Background,
    /// 一键清理的执行权（在连接线程里执行，见 [`ManualClean`]）。
    manual: ManualSlot,
}

/// 准备好的一键清理：要执行的步骤（在引擎锁外执行），以及已经知道的结果（延后的、提醒）。
/// 执行完之前占着执行权（`turn`）。
pub struct ManualClean {
    pub job: CleanupJob,
    pub summary: CleanupResult,
    pub turn: ManualTurn,
}

/// 服务停止时最多等正在执行的一键清理多久。
const MANUAL_CLEAN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// 自动清理的最小间隔。
const AUTO_CLEAN_EVERY_MS: Millis = 10 * 60_000;
/// 整机 CPU 超过这个比例时不做自动清理，避免清理本身造成卡顿。
const AUTO_CLEAN_MAX_CPU: f32 = 50.0;
/// 健康状态用的清理预演多久重新算一次。
const PREVIEW_EVERY_MS: Millis = 10_000;
/// 泄漏嫌疑与异常增长的重新计算间隔。
const SUSPECTS_EVERY_MS: Millis = 60_000;

impl Engine {
    /// 前台有游戏、全屏程序，或者正在用的程序在持续干重活（重点程序）：会打扰的清理延后，自动清理暂停。
    fn foreground_immersive(&self) -> bool {
        let now = self.now();
        self.session.current(now).fullscreen
            || self
                .view
                .as_ref()
                .is_some_and(|v| v.game.is_some() || v.focused)
    }

    /// 每个采样周期调用：到时间就重新计算泄漏嫌疑和异常增长。
    pub(super) fn refresh_suspects(&mut self, snapshot: &Snapshot, now: Millis) {
        let due = self
            .cleanup
            .suspects_at
            .is_none_or(|t| now.saturating_sub(t) >= SUSPECTS_EVERY_MS);
        if due {
            self.cleanup.suspects = self.leaks.suspects();
            self.cleanup.grown = self.growth.grown(snapshot, now);
            self.cleanup.suspects_at = Some(now);
        }
    }

    /// 按当前状态生成清理计划。所有清理（以及健康评估的预演）都经过这里。
    pub(super) fn plan_cleanup(
        &self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        trigger: Trigger,
        foreground_immersive: bool,
    ) -> CleanupPlan {
        cleanup::plan(
            &CleanupConfig::default(),
            &PlanInput {
                snapshot,
                classes,
                suspects: &self.cleanup.suspects,
                grown: &self.cleanup.grown,
                trigger,
                foreground_immersive,
                restart_advised: self.kernel_pool.restart_advised(),
            },
        )
    }

    /// 健康状态用的"手动清理"预演：只用来统计可清理的项和是否建议重启。它依赖的泄漏嫌疑、异常增长
    /// 每分钟才更新一次，内存变化也慢，所以每隔一会儿才重新预演，不是每轮都算。
    pub(super) fn cleanup_preview(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        now: Millis,
    ) -> &CleanupPlan {
        let stale = self
            .cleanup
            .preview
            .as_ref()
            .is_none_or(|(at, _)| now.saturating_sub(*at) >= PREVIEW_EVERY_MS);
        if stale {
            let plan = self.plan_cleanup(snapshot, classes, Trigger::Manual, false);
            self.cleanup.preview = Some((now, plan));
        }
        &self.cleanup.preview.as_ref().expect("刚刚算过").1
    }

    /// 用户点了"一键清理"：全部可清理项立即执行（交给 [`CleanupJob`] 在引擎锁外执行，执行完再交给
    /// [`Engine::finish_clean_now`]）；前台有游戏或全屏程序时，会打扰的项目留到之后。
    /// 服务正在停止、还没有采样结果时为空；上一次一键清理还在执行时报错。
    pub fn begin_clean_now(&mut self) -> Result<Option<ManualClean>, RequestError> {
        if self.stopped {
            return Ok(None);
        }
        let Some(turn) = self.cleanup.manual.take() else {
            return Err(RequestError::new(
                ErrorCode::Failed,
                "正在清理，请等这一次做完",
            ));
        };
        self.answers.invalidate();
        let immersive = self.foreground_immersive();
        let Some(view) = self.view.as_ref() else {
            return Ok(None);
        };
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Manual, immersive);
        let summary = CleanupResult {
            deferred: plan.deferred.len(),
            advisories: plan.advisories.iter().map(convert::advisory).collect(),
            ..Default::default()
        };
        let job = self.cleanup_job(plan.run_now, Trigger::Manual);
        self.cleanup.deferred = plan.deferred;
        Ok(Some(ManualClean { job, summary, turn }))
    }

    /// 服务停止前等正在执行的一键清理做完（最多几秒），不在它重启程序的半途撤销调整。
    pub(super) fn wait_manual_clean(&self) {
        if !self.cleanup.manual.wait_idle(MANUAL_CLEAN_WAIT) {
            log::warn!(
                "一键清理没在 {} 秒内做完，照常停止",
                MANUAL_CLEAN_WAIT.as_secs()
            );
        }
    }

    /// 一键清理执行完：记下结果。
    pub fn finish_clean_now(
        &mut self,
        mut result: CleanupResult,
        done: CleanupDone,
    ) -> CleanupResult {
        self.answers.invalidate();
        for (step, outcome) in &done.outcomes {
            match outcome {
                Ok(_) => {
                    if !self.cfg.dry_run {
                        result.freed_bytes += step.memory_bytes;
                    }
                    result.handled.push(step.name.clone());
                    log::info!("清理：{}（{:?}）", step.name, step.action);
                }
                Err(e) => {
                    result.failed.push(step.name.clone());
                    log::warn!("清理 {} 失败：{e}", step.name);
                }
            }
        }
        self.record_cleanup(
            CleanupTrigger::Manual,
            result.freed_bytes,
            result.handled.len(),
            result.failed.len(),
        );
        // 清理过的进程不再沿用旧的泄漏判断，等下一轮重新计算。
        self.cleanup.suspects.clear();
        self.cleanup.grown.clear();
        self.cleanup.suspects_at = None;
        result
    }

    /// 清理页：与点"一键清理"时相同的计划，按程序列出。
    pub fn cleanup_overview(&self) -> Option<CleanupOverview> {
        let view = self.view.as_ref()?;
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Manual, false);
        let cpus = view.snapshot.system.logical_cpus;
        let cpu_pct: HashMap<Pid, f32> = view
            .snapshot
            .processes
            .iter()
            .map(|p| {
                (
                    p.facts.key.pid,
                    self.activity.usage(&p.facts.key).share_pct(cpus),
                )
            })
            .collect();
        let processes = cleanup::overview(&OverviewInput {
            snapshot: &view.snapshot,
            classes: &view.classes,
            plan: &plan,
            suspects: &self.cleanup.suspects,
            grown: &self.cleanup.grown,
            cpu_pct: &cpu_pct,
        });
        let sys = &view.snapshot.system;
        Some(CleanupOverview {
            freeable_bytes: plan.total_freeable(),
            needs_attention: processes.iter().filter(|p| p.needs_attention()).count(),
            mem_used_bytes: sys.mem_used_bytes(),
            mem_total_bytes: sys.mem_total,
            restart_advised: self.kernel_pool.restart_advised(),
            auto_clean: self.settings.auto_clean,
            last_cleanup: self.last_cleanup(),
            since_boot: self.since_boot(&view.snapshot),
            processes,
        })
    }

    /// 把清理步骤和当前的采样核对（进程必须还是同一个，PID 未被复用），准备在锁外执行。
    fn cleanup_job(&self, steps: Vec<CleanupStep>, trigger: Trigger) -> CleanupJob {
        let steps = steps
            .into_iter()
            .map(|step| {
                let target = self.view.as_ref().and_then(|view| {
                    let current = view
                        .snapshot
                        .find(step.key.pid)
                        .filter(|p| p.facts.key == step.key)?;
                    let class = view.classes.get(&step.key.pid)?;
                    Some((class.clone(), current.facts.priority.unwrap_or_default()))
                });
                ReadyStep { step, target }
            })
            .collect();
        CleanupJob {
            executor: self.executor.clone(),
            trigger,
            steps,
        }
    }

    /// 用户点过清理、但当时前台有游戏或全屏程序而延后的项目：退出后在单独的线程里执行。
    pub(super) fn run_deferred_cleanup(&mut self) {
        if self.cleanup.deferred.is_empty()
            || self.cleanup.background.busy()
            || self.foreground_immersive()
        {
            return;
        }
        let steps = std::mem::take(&mut self.cleanup.deferred);
        let job = self.cleanup_job(steps, Trigger::Manual);
        self.cleanup.background.start(job);
    }

    /// 自动清理：只做无感项目，一次只处理一个，游戏/全屏/高负载时不做。在单独的线程里执行。
    pub(super) fn auto_clean(&mut self, now: Millis) {
        if !self.settings.auto_clean
            || self.cleanup.background.busy()
            || now.saturating_sub(self.cleanup.last_auto_at) < AUTO_CLEAN_EVERY_MS
            || self.foreground_immersive()
            || self.activity.system_cpu_pct() > AUTO_CLEAN_MAX_CPU
        {
            return;
        }
        self.cleanup.last_auto_at = now;
        let Some(view) = &self.view else { return };
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Auto, false);
        let Some(step) = plan.run_now.into_iter().next() else {
            return;
        };
        let job = self.cleanup_job(vec![step], Trigger::Auto);
        self.cleanup.background.start(job);
    }

    /// 取回在单独线程里执行完的清理，记下结果。
    pub(super) fn collect_cleanup(&mut self) {
        let Some(done) = self.cleanup.background.finished() else {
            return;
        };
        let (trigger, verb) = match done.trigger {
            Trigger::Auto => (CleanupTrigger::Auto, "自动清理"),
            _ => (CleanupTrigger::Deferred, "延后的清理项"),
        };
        let (mut freed, mut handled) = (0, 0);
        for (step, outcome) in &done.outcomes {
            match outcome {
                Ok(_) => {
                    log::info!("{verb}：{}", step.name);
                    freed += step.memory_bytes;
                    handled += 1;
                }
                Err(e) => log::info!("{verb} {} 未执行：{e}", step.name),
            }
        }
        self.record_cleanup(trigger, freed, handled, 0);
    }

    /// 记录真正做了事的清理（观察模式下什么都没做，不记录），并记一条日志事件。
    fn record_cleanup(
        &mut self,
        trigger: CleanupTrigger,
        freed_bytes: u64,
        handled: usize,
        failed: usize,
    ) {
        if handled == 0 || self.cfg.dry_run {
            return;
        }
        self.logbook.event(Event::Cleaned {
            trigger,
            freed_bytes,
            handled: handled as u32,
            failed: failed as u32,
        });
        self.cleanup.last = Some(LastCleanup {
            at_unix_ms: unix_now_ms(),
            trigger,
            freed_bytes,
            handled,
        });
    }

    /// 泄漏嫌疑（卡顿诊断用）。
    pub(super) fn leak_suspects(&self) -> &[LeakSuspect] {
        &self.cleanup.suspects
    }

    pub(super) fn deferred_count(&self) -> usize {
        self.cleanup.deferred.len()
    }

    pub(super) fn last_cleanup(&self) -> Option<LastCleanup> {
        self.cleanup.last
    }
}
