//! 采样周期：采集 → 分类 → 调度 → 硬件模式 → 健康评估 → 自动清理。

use std::collections::HashMap;
use std::time::Duration;

use poise_core::classify::{Classification, ClassifyInput};
use poise_core::diagnose::{self, DiagnoseConfig, HealthInput, Heat};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::prefs::Mode;
use poise_core::safety::Trigger;
use poise_core::schedule::{GameInfo, ScheduleInput};

use super::{Engine, View};

/// 采样间隔。前台切换不等采样，会立即触发一次调度。
pub const TICK: Duration = Duration::from_secs(2);
/// 泄漏嫌疑的重新计算间隔。
const SUSPECTS_EVERY_MS: Millis = 60_000;

struct ScheduleOutcome {
    effective_mode: Mode,
    game: Option<GameInfo>,
}

impl Engine {
    pub fn tick(&mut self) {
        let now = self.now();
        let snapshot = match self.snapshotter.take(now) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("采样失败：{e}");
                return;
            }
        };
        self.activity.update(&snapshot);
        self.leaks.observe(&snapshot);
        self.kernel_pool
            .observe(now, snapshot.system.kernel_nonpaged);
        self.refresh_suspects(now);

        let classes = self.classify(&snapshot, now);
        let outcome = self.schedule(&snapshot, &classes);
        let reading = self
            .hardware
            .read(outcome.game.is_some() || self.ui_active(now), now);
        let heat = self
            .heat
            .observe(&DiagnoseConfig::default(), reading.thermals, now);
        let health = self.assess(&snapshot, &classes, &outcome, &reading, heat);
        self.apply_hardware_mode(outcome.effective_mode, &health, now);

        self.view = Some(View {
            snapshot,
            classes,
            effective_mode: outcome.effective_mode,
            game: outcome.game,
            reading,
            health,
        });
        self.run_deferred_cleanup();
        self.auto_clean(now);
    }

    /// 前台切换时立即用上一轮的快照重新调度，不等下一次采样。
    pub fn reschedule_now(&mut self) {
        let now = self.now();
        let Some(view) = self.view.take() else {
            return;
        };
        let classes = self.classify(&view.snapshot, now);
        let outcome = self.schedule(&view.snapshot, &classes);
        self.view = Some(View {
            classes,
            effective_mode: outcome.effective_mode,
            game: outcome.game,
            ..view
        });
    }

    /// 界面正显示着才读显卡（在 Optimus 笔记本上读显卡会唤醒休眠中的独显）。
    fn ui_active(&self, now: Millis) -> bool {
        self.session.ui_visible(now)
    }

    pub(super) fn foreground_immersive(&self) -> bool {
        let now = self.now();
        self.session.current(now).fullscreen || self.view.as_ref().is_some_and(|v| v.game.is_some())
    }

    fn refresh_suspects(&mut self, now: Millis) {
        let due = self
            .cleanup
            .suspects_at
            .is_none_or(|t| now.saturating_sub(t) >= SUSPECTS_EVERY_MS);
        if due {
            self.cleanup.suspects = self.leaks.suspects();
            self.cleanup.suspects_at = Some(now);
        }
    }

    fn classify(&self, snapshot: &Snapshot, now: Millis) -> HashMap<Pid, Classification> {
        let own: Vec<Pid> = self.own_pids.iter().copied().collect();
        self.classifier.classify_all(&ClassifyInput {
            snapshot,
            session: &self.session.current(now),
            activity: &self.activity,
            preferences: &self.settings.preferences,
            own_pids: &own,
            session_fresh: self.session.is_fresh(now),
        })
    }

    fn schedule(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
    ) -> ScheduleOutcome {
        let session = self.session.current(self.now());
        let plan = self.scheduler.plan(&ScheduleInput {
            snapshot,
            session: &session,
            classes,
            activity: &self.activity,
            user_mode: self.settings.mode,
            auto_full_for_games: self.settings.auto_full_for_games,
        });
        for planned in &plan.actions {
            let Some(class) = classes.get(&planned.key.pid) else {
                continue;
            };
            let result = self.executor.execute(
                &planned.action,
                class,
                planned.original_priority,
                Trigger::Auto,
            );
            match &result {
                Ok(_) if self.cfg.dry_run => log::info!("[观察] {:?}", planned.action),
                Ok(_) => log::debug!("{:?}", planned.action),
                Err(e) => log::debug!("{:?} 未执行：{e}", planned.action),
            }
            self.scheduler.report(planned, &result);
        }
        ScheduleOutcome {
            effective_mode: plan.effective_mode,
            game: plan.game,
        }
    }

    fn assess(
        &self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        outcome: &ScheduleOutcome,
        reading: &poise_hardware::HardwareReading,
        heat: Heat,
    ) -> poise_core::diagnose::Health {
        let preview = self.plan_cleanup(snapshot, classes, Trigger::Manual, false);
        diagnose::assess(
            &DiagnoseConfig::default(),
            &HealthInput {
                system: &snapshot.system,
                cpu_pct: self.activity.system_cpu_pct(),
                heat,
                game: outcome.game.as_ref(),
                manual_plan: &preview,
                system_drive: reading.storage.system_drive,
            },
        )
    }
}
