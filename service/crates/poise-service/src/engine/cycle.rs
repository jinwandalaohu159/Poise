//! 采样周期：采集 → 观察趋势 → 分类 → 调度 → 读取传感器 → 健康评估 → 硬件模式 → 自动清理。

use std::collections::HashMap;
use std::time::Duration;

use poise_core::classify::{Classification, ClassifyInput};
use poise_core::diagnose::{self, DiagnoseConfig, Health, HealthInput, Sample};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::safety::Trigger;
use poise_hardware::HardwareReading;

use super::schedule::ScheduleOutcome;
use super::{Engine, View};

/// 采样间隔。前台切换不等采样，会立即触发一次调度。
pub const TICK: Duration = Duration::from_secs(2);

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
        self.observe(&snapshot, now);
        let classes = self.classify(&snapshot, now);
        let outcome = self.schedule(&snapshot, &classes);
        let reading = self.read_sensors(outcome.game.is_some(), now);
        let health = self.assess(&snapshot, &classes, &outcome, &reading, now);
        self.record_history(&snapshot, &reading, now);
        let effective_mode = self.apply_modes(outcome.effective_mode, &health, now);
        self.view = Some(View {
            snapshot,
            classes,
            effective_mode,
            game: outcome.game,
            reading,
            health,
        });
        self.remember_running_apps();
        self.run_deferred_cleanup();
        self.auto_clean(now);
    }

    /// 前台切换、用户改模式时立即重新调度并落到硬件，不等下一次采样。
    /// 重新取一次进程快照（约 10 ms）：刚打开的程序也能立即拿到前台待遇。
    /// 这样紧接着的状态查询就是最终结果（包括过热降级后的实际模式），界面不会先显示一个中间状态。
    pub fn reschedule_now(&mut self) {
        let now = self.now();
        let Some(view) = self.view.take() else {
            return;
        };
        let snapshot = self.snapshotter.take(now).unwrap_or(view.snapshot);
        let classes = self.classify(&snapshot, now);
        let outcome = self.schedule(&snapshot, &classes);
        let effective_mode = self.apply_modes(outcome.effective_mode, &view.health, now);
        self.view = Some(View {
            snapshot,
            classes,
            effective_mode,
            game: outcome.game,
            reading: view.reading,
            health: view.health,
        });
    }

    /// 只在采样周期里做的观察：CPU 占用、泄漏与增长趋势。
    fn observe(&mut self, snapshot: &Snapshot, now: Millis) {
        self.activity.update(snapshot);
        self.leaks.observe(snapshot);
        self.kernel_pool
            .observe(now, snapshot.system.kernel_nonpaged);
        self.observe_growth(snapshot, now);
        self.refresh_suspects(snapshot, now);
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

    /// 取采集线程最近一次的读数（不等待）。界面显示着或在玩游戏时才读显卡和风扇
    /// （在 Optimus 笔记本上读显卡会唤醒休眠中的独显）。
    fn read_sensors(&mut self, gaming: bool, now: Millis) -> HardwareReading {
        self.sensors
            .set_detailed(gaming || self.session.ui_visible(now));
        self.sensors.latest()
    }

    fn assess(
        &mut self,
        snapshot: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        outcome: &ScheduleOutcome,
        reading: &HardwareReading,
        now: Millis,
    ) -> Health {
        let heat = self
            .heat
            .observe(&DiagnoseConfig::default(), reading.thermals, now);
        self.last_heat = heat;
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

    /// 走势图：整机 CPU、大核 / 小核负载、内存、CPU 温度。
    fn record_history(&mut self, snapshot: &Snapshot, reading: &HardwareReading, now: Millis) {
        let cores = self.core_load_meter.sample();
        self.history.push(
            now,
            Sample {
                cpu_pct: self.activity.system_cpu_pct(),
                performance_pct: cores.map(|c| c.performance_pct),
                efficiency_pct: cores.map(|c| c.efficiency_pct),
                mem_pct: snapshot.system.mem_used_pct(),
                cpu_temp: reading.thermals.cpu_temp,
            },
        );
    }
}
