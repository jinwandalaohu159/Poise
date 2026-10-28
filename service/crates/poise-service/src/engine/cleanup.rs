//! 一键清理、延后清理、自动清理，以及最近一次清理的记录。

use std::collections::HashMap;

use poise_core::action::{ExecError, ExecOutcome};
use poise_core::classify::Classification;
use poise_core::cleanup::{
    self, CleanupConfig, CleanupPlan, CleanupStep, OverviewInput, PlanInput,
};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::safety::Trigger;
use poise_protocol::{CleanupOverview, CleanupResult, CleanupTrigger, LastCleanup};

use super::{Engine, convert, unix_now_ms};

/// 自动清理的最小间隔。
const AUTO_CLEAN_EVERY_MS: Millis = 10 * 60_000;
/// 整机 CPU 超过这个比例时不做自动清理，避免清理本身造成卡顿。
const AUTO_CLEAN_MAX_CPU: f32 = 50.0;
/// 泄漏嫌疑与异常增长的重新计算间隔。
const SUSPECTS_EVERY_MS: Millis = 60_000;

impl Engine {
    /// 前台有游戏或全屏程序：会打扰的清理延后，自动清理暂停。
    fn foreground_immersive(&self) -> bool {
        let now = self.now();
        self.session.current(now).fullscreen || self.view.as_ref().is_some_and(|v| v.game.is_some())
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

    /// 用户点了"一键清理"：立即执行全部可清理项；前台有游戏或全屏程序时，会打扰的项目留到之后。
    pub fn clean_now(&mut self) -> CleanupResult {
        let immersive = self.foreground_immersive();
        let Some(view) = &self.view else {
            return CleanupResult::default();
        };
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Manual, immersive);
        let mut result = CleanupResult {
            deferred: plan.deferred.len(),
            advisories: plan.advisories.iter().map(convert::advisory).collect(),
            ..Default::default()
        };
        for step in &plan.run_now {
            match self.execute_step(step, Trigger::Manual) {
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
        );
        self.cleanup.deferred = plan.deferred;
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

    /// 执行一个清理步骤。执行前重新核对：进程必须还是同一个（PID 未被复用）。
    fn execute_step(&self, step: &CleanupStep, trigger: Trigger) -> Result<ExecOutcome, ExecError> {
        let view = self.view.as_ref().ok_or(ExecError::Gone)?;
        let current = view
            .snapshot
            .find(step.key.pid)
            .filter(|p| p.facts.key == step.key)
            .ok_or(ExecError::Gone)?;
        let class = view.classes.get(&step.key.pid).ok_or(ExecError::Gone)?;
        self.executor.execute(
            &step.action,
            class,
            current.facts.priority.unwrap_or_default(),
            trigger,
        )
    }

    /// 用户点过清理、但当时前台有游戏或全屏程序而延后的项目：退出后执行。
    pub(super) fn run_deferred_cleanup(&mut self) {
        if self.cleanup.deferred.is_empty() || self.foreground_immersive() {
            return;
        }
        let steps = std::mem::take(&mut self.cleanup.deferred);
        let (mut freed, mut handled) = (0, 0);
        for step in &steps {
            match self.execute_step(step, Trigger::Manual) {
                Ok(_) => {
                    log::info!("已处理延后的清理项：{}", step.name);
                    freed += step.memory_bytes;
                    handled += 1;
                }
                Err(e) => log::info!("延后的清理项 {} 未执行：{e}", step.name),
            }
        }
        self.record_cleanup(CleanupTrigger::Deferred, freed, handled);
    }

    /// 自动清理：只做无感项目，一次只处理一个，游戏/全屏/高负载时不做。
    pub(super) fn auto_clean(&mut self, now: Millis) {
        if !self.settings.auto_clean
            || now.saturating_sub(self.cleanup.last_auto_at) < AUTO_CLEAN_EVERY_MS
            || self.foreground_immersive()
            || self.activity.system_cpu_pct() > AUTO_CLEAN_MAX_CPU
        {
            return;
        }
        self.cleanup.last_auto_at = now;
        let Some(view) = &self.view else { return };
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Auto, false);
        let Some(step) = plan.run_now.first() else {
            return;
        };
        match self.execute_step(step, Trigger::Auto) {
            Ok(_) => {
                log::info!("自动清理：{}", step.name);
                self.record_cleanup(CleanupTrigger::Auto, step.memory_bytes, 1);
            }
            Err(e) => log::info!("自动清理 {} 未执行：{e}", step.name),
        }
    }

    /// 记录真正做了事的清理（观察模式下什么都没做，不记录）。
    fn record_cleanup(&mut self, trigger: CleanupTrigger, freed_bytes: u64, handled: usize) {
        if handled == 0 || self.cfg.dry_run {
            return;
        }
        self.cleanup.last = Some(LastCleanup {
            at_unix_ms: unix_now_ms(),
            trigger,
            freed_bytes,
            handled,
        });
    }

    pub(super) fn deferred_count(&self) -> usize {
        self.cleanup.deferred.len()
    }

    pub(super) fn last_cleanup(&self) -> Option<LastCleanup> {
        self.cleanup.last
    }
}
