//! 采样周期：（锁外采集，见 `sampler`）→ 观察趋势 → 分类 → 调度 → 读取传感器 → 健康评估 → 硬件模式 →
//! 检查电源设置 → 自动清理。
//! 托盘代理不在时（Poise 已退出）只采样，不调度、不切换模式、不清理（见 `presence`）。
//! 多久采一轮见 `cadence`；前台切换、全屏切换不等采样，立即重新调度（[`Engine::reschedule_now`]）。

use std::collections::HashMap;

use poise_core::classify::{Classification, ClassifyInput};
use poise_core::diagnose::{self, DiagnoseConfig, Health, HealthInput, Sample};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_hardware::{HardwareReading, SensorLevel};
use poise_protocol::Event;

use super::phases::{Phases, TickCost};
use super::sampler::Sampled;
use super::schedule::ScheduleOutcome;
use super::{Engine, View};

/// 两轮采样之间隔了这么久：电脑睡眠过（平时最长 6 秒一轮）。
const SLEPT_AFTER_MS: Millis = 15_000;

impl Engine {
    /// 处理一轮采样（快照已在引擎锁外采集，见 [`super::Sampler`]），返回各步的耗时（见 [`TickCost`]）。
    pub fn tick(&mut self, sample: Sampled) -> TickCost {
        let mut phases = Phases::start();
        phases.outside("采集进程", sample.spent);
        if self.stopped {
            return phases.finish();
        }
        self.answers.invalidate();
        let Some(snapshot) = sample.snapshot else {
            return phases.finish();
        };
        // 采集期间立即重新调度时取了更新的快照（见 [`Engine::reschedule_now`]）：这份已经过时，不用。
        if self
            .view
            .as_ref()
            .is_some_and(|v| v.snapshot.at > snapshot.at)
        {
            return phases.finish();
        }
        let now = self.now();
        if self
            .last_tick_at
            .replace(now)
            .is_some_and(|last| now.saturating_sub(last) >= SLEPT_AFTER_MS)
        {
            self.session.resumed(now);
        }
        self.observe(&snapshot, now);
        phases.done("观察趋势");
        let active = self.ensure_presence(now);
        phases.done("托盘代理状态");
        let classes = self.classify(&snapshot, now);
        phases.done("分类");
        self.restore_leftovers(&snapshot, &classes);
        // 上一轮的结果：日志里给刚退出的程序取名字用。
        let previous = self.view.take();
        let outcome = self.schedule_if(
            active,
            &snapshot,
            previous.as_ref().map(|v| &v.snapshot),
            &classes,
        );
        phases.done("调度");
        let reading = self.read_sensors(&outcome, now);
        let health = self.assess(&snapshot, &classes, &outcome, &reading, now);
        self.record_history(&snapshot, &reading, now);
        phases.done("健康评估");
        let effective_mode = if active {
            self.apply_modes(outcome.effective_mode, snapshot.system.on_battery, now)
        } else {
            outcome.effective_mode
        };
        phases.done("硬件模式");
        self.view = Some(View {
            snapshot,
            classes,
            effective_mode,
            game: outcome.game,
            focused: outcome.focused,
            reading,
            health,
        });
        self.report_round();
        self.remember_running_apps(now);
        phases.done("记录运行的程序");
        self.guard_power(now);
        self.changes.settle();
        phases.done("检查电源设置");
        self.collect_cleanup();
        if active {
            self.run_deferred_cleanup();
            self.auto_clean(now);
        }
        phases.done("清理");
        let cost = phases.finish();
        self.cost_log.add(&cost);
        cost
    }

    /// 前台切换、用户改模式时立即重新调度并落到硬件，不等下一次采样。
    /// 新的前台程序已经在上一轮采样里（切换到已经开着的程序，最常见）时直接沿用上一轮的采样，
    /// 响应更快；刚打开的程序还不在里面，才重新取一次进程快照（约 5 毫秒），让它也立即拿到前台待遇。
    /// 这样紧接着的状态查询就是最终结果（包括实际生效的模式），界面不会先显示一个中间状态。
    pub fn reschedule_now(&mut self) {
        if self.stopped {
            return;
        }
        self.answers.invalidate();
        let now = self.now();
        self.last_change_at = now;
        if !self.ensure_presence(now) {
            return;
        }
        let Some(view) = self.view.take() else {
            return;
        };
        let foreground = self.session.current(now).foreground_pid;
        let known = foreground.is_none_or(|pid| view.snapshot.find(pid).is_some());
        let fresh = if known { None } else { self.sampler.take(now) };
        let (snapshot, previous) = match fresh {
            Some(fresh) => (fresh, Some(view.snapshot)),
            None => (view.snapshot, None),
        };
        let classes = self.classify(&snapshot, now);
        let outcome = self.schedule_if(true, &snapshot, previous.as_ref(), &classes);
        let effective_mode =
            self.apply_modes(outcome.effective_mode, snapshot.system.on_battery, now);
        self.view = Some(View {
            snapshot,
            classes,
            effective_mode,
            game: outcome.game,
            focused: outcome.focused,
            reading: view.reading,
            health: view.health,
        });
    }

    /// 只在采样周期里做的观察：CPU 占用、泄漏与增长趋势。
    fn observe(&mut self, snapshot: &Snapshot, now: Millis) {
        self.activity.update(snapshot);
        self.measure_foreground_gpu(now);
        self.leaks.observe(snapshot);
        self.kernel_pool
            .observe(now, snapshot.system.kernel_nonpaged);
        self.observe_growth(snapshot, now);
        self.refresh_suspects(snapshot, now);
    }

    /// 前台全屏时测量它的显卡 3D 占用（识别游戏用）；不全屏时停止测量。
    /// 测量在独立线程里进行，这里只取最近一次的结果；立即重新调度时沿用上次的结果。
    /// 已经确认为游戏的不再测量：读取显卡计数器要查询显卡驱动，没必要在游戏运行时一直做。
    fn measure_foreground_gpu(&mut self, now: Millis) {
        let session = self.session.current(now);
        let games = self.scheduler.active_games();
        let target = session
            .foreground_pid
            .filter(|pid| session.fullscreen && !games.contains(pid));
        self.gpu.measure(target);
        self.foreground_gpu = self.gpu.latest();
    }

    pub(super) fn classify(
        &mut self,
        snapshot: &Snapshot,
        now: Millis,
    ) -> HashMap<Pid, Classification> {
        let own = self.clients.own_pids();
        let games = self.scheduler.active_games();
        let session = self.session.current(now);
        self.classifier.classify_all(&ClassifyInput {
            snapshot,
            session: &session,
            activity: &self.activity,
            preferences: &self.settings.preferences,
            own_pids: &own,
            session_fresh: self.session.is_fresh(now),
            foreground_gpu: self.foreground_gpu,
            active_games: &games,
        })
    }

    /// 取采集线程最近一次的读数（不等待），并告诉它需要的详细程度（见 [`SensorLevel::choose`]）：
    /// 玩游戏、有重点程序时少调用或不调用机型接口（可能让所有核心短暂停顿），界面显示着也一样；
    /// 没有要紧的事时界面显示着才全读，其余时候不访问显卡（在 Optimus 笔记本上会唤醒休眠中的独显）。
    fn read_sensors(&mut self, outcome: &ScheduleOutcome, now: Millis) -> HardwareReading {
        self.sensors.set_level(SensorLevel::choose(
            self.session.ui_visible(now),
            outcome.game.is_some(),
            outcome.focused,
        ));
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
        let cfg = DiagnoseConfig::default();
        let (headroom, limited_changed) =
            self.headroom
                .observe(&cfg, reading.thermals, &snapshot.system, now);
        match limited_changed {
            Some(true) => self.logbook.event(Event::Throttled {
                limit_pct: headroom.limit_pct,
            }),
            Some(false) => self.logbook.event(Event::Unthrottled),
            None => {}
        }
        let cpu_pct = self.activity.system_cpu_pct();
        let preview = self.cleanup_preview(snapshot, classes, now);
        diagnose::assess(
            &cfg,
            &HealthInput {
                system: &snapshot.system,
                cpu_pct,
                heat: headroom.heat,
                game: outcome.game.as_ref(),
                manual_plan: preview,
                system_drive: reading.storage.system_drive,
            },
        )
    }

    /// 走势图：整机 CPU、大核 / 小核负载、内存、CPU 温度。
    fn record_history(&mut self, snapshot: &Snapshot, reading: &HardwareReading, now: Millis) {
        let classes = self
            .activity
            .class_loads(&self.topology.performance_by_index);
        self.history.push(
            now,
            Sample {
                cpu_pct: self.activity.system_cpu_pct(),
                performance_pct: classes.map(|c| c.0),
                efficiency_pct: classes.map(|c| c.1),
                mem_pct: snapshot.system.mem_used_pct(),
                cpu_temp: reading.thermals.cpu_temp,
            },
        );
    }
}
