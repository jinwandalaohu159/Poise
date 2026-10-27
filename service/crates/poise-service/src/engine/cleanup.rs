//! 一键清理、延后清理、自动清理，以及最近一次清理的记录。

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use poise_core::action::{ExecError, ExecOutcome};
use poise_core::classify::Classification;
use poise_core::cleanup::{self, Advisory, CleanupConfig, CleanupPlan, CleanupStep, PlanInput};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::safety::Trigger;
use poise_protocol::{AdvisoryDto, CleanupResult, CleanupTrigger, LastCleanup};

use super::Engine;

/// 自动清理的最小间隔。
const AUTO_CLEAN_EVERY_MS: Millis = 10 * 60_000;
/// 整机 CPU 超过这个比例时不做自动清理，避免清理本身造成卡顿。
const AUTO_CLEAN_MAX_CPU: f32 = 50.0;

impl Engine {
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
                trigger,
                foreground_immersive,
                restart_advised: self.kernel_pool.restart_advised(),
            },
        )
    }

    /// 用户点了"一键清理"：立即执行全部可清理项；前台有游戏时，会打扰的项目留到之后。
    pub fn clean_now(&mut self) -> CleanupResult {
        let immersive = self.foreground_immersive();
        let Some(view) = &self.view else {
            return CleanupResult::default();
        };
        let plan = self.plan_cleanup(&view.snapshot, &view.classes, Trigger::Manual, immersive);
        let mut result = CleanupResult {
            deferred: plan.deferred.len(),
            advisories: plan.advisories.iter().map(to_dto).collect(),
            ..Default::default()
        };
        for step in &plan.run_now {
            match self.execute_step(step, Trigger::Manual) {
                Ok(_) => {
                    if !self.cfg.dry_run {
                        result.freed_bytes += step.private_bytes;
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
        self.cleanup.suspects_at = None;
        result
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

    /// 用户点过清理、但当时在玩游戏而延后的项目：游戏结束后执行。
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
                    freed += step.private_bytes;
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
                self.record_cleanup(CleanupTrigger::Auto, step.private_bytes, 1);
            }
            Err(e) => log::info!("自动清理 {} 未执行：{e}", step.name),
        }
    }

    /// 记录真正做了事的清理（观察模式下什么都没做，不记录）。
    fn record_cleanup(&mut self, trigger: CleanupTrigger, freed_bytes: u64, handled: usize) {
        if handled == 0 || self.cfg.dry_run {
            return;
        }
        let at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.cleanup.last = Some(LastCleanup {
            at_unix_ms,
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

fn to_dto(a: &Advisory) -> AdvisoryDto {
    match a {
        Advisory::HeavyApp {
            name,
            private_bytes,
            ..
        } => AdvisoryDto::HeavyApp {
            name: name.clone(),
            private_bytes: *private_bytes,
        },
        Advisory::RestartRecommended { .. } => AdvisoryDto::RestartRecommended,
        Advisory::ManualPending { count } => AdvisoryDto::ManualPending { count: *count },
    }
}
