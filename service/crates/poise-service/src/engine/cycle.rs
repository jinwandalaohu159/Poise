//! 采样周期：采集 → 观察趋势 → 分类 → 调度 → 读取传感器 → 健康评估 → 硬件模式 → 检查电源设置 → 自动清理。
//! 托盘代理不在时（Poise 已退出）只采样，不调度、不切换模式、不清理（见 `presence`）。

use std::collections::HashMap;
use std::time::Duration;

use poise_core::classify::{Classification, ClassifyInput};
use poise_core::diagnose::{self, DiagnoseConfig, Health, HealthInput, Sample};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_hardware::{HardwareReading, SensorLevel};

use super::phases::Phases;
use super::schedule::ScheduleOutcome;
use super::{Engine, View};

/// 采样间隔。前台切换不等采样，会立即触发一次调度。
pub const TICK: Duration = Duration::from_secs(2);

/// 平静时的采样间隔：一段时间里没有调整任何程序、没有新的决定，也没有游戏、重点程序、全屏程序、
/// 打开的界面（见 [`Engine::next_tick_in`]）。采集进程表是每轮最贵的一步（系统要列出所有线程），
/// 这时少采一半；前台切换照样立即调度，不受影响。
const CALM_TICK: Duration = Duration::from_secs(4);

/// 最近一次变化（调整了程序、决定变了、前台切换）之后多久算平静。
const CALM_AFTER_MS: Millis = 20_000;

/// 没人在用电脑时的采样间隔（见 [`Engine::next_tick_in`]）。
const IDLE_TICK: Duration = Duration::from_secs(6);

/// 用户多久不操作算离开。
const AWAY_AFTER_MS: Millis = 2 * 60_000;

impl Engine {
    pub fn tick(&mut self) {
        let mut phases = Phases::start();
        let now = self.now();
        let snapshot = match self.snapshotter.take(now) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("采样失败：{e}");
                return;
            }
        };
        phases.done("采集进程");
        self.observe(&snapshot, now);
        phases.done("观察趋势");
        let active = self.ensure_presence(now);
        phases.done("托盘代理状态");
        let classes = self.classify(&snapshot, now);
        phases.done("分类");
        self.restore_leftovers(&snapshot, &classes);
        let outcome = self.schedule_if(active, &snapshot, &classes);
        phases.done("调度");
        let reading = self.read_sensors(&outcome, now);
        let health = self.assess(&snapshot, &classes, &outcome, &reading, now);
        self.record_history(&snapshot, &reading, now);
        phases.done("健康评估");
        let effective_mode = if active {
            self.apply_modes(outcome.effective_mode, now)
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
        self.remember_running_apps(now);
        phases.done("记录运行的程序");
        self.guard_power(now);
        phases.done("检查电源设置");
        if active {
            self.run_deferred_cleanup();
            self.auto_clean(now);
        }
        phases.done("清理");
        phases.finish();
    }

    /// 下一轮采样前等多久。用户离开、没在玩游戏、界面也没显示时放慢到 [`IDLE_TICK`]；平静时
    /// 放慢到 [`CALM_TICK`]；其余时候 [`TICK`]。用户一有动作（前台变化）就会立即重新调度，不受影响。
    /// 所有判断都按时间而不是轮数，间隔变了结果不变。
    pub fn next_tick_in(&self) -> Duration {
        let now = self.now();
        let session = self.session.current(now);
        let (gaming, focused) = self
            .view
            .as_ref()
            .map_or((false, false), |v| (v.game.is_some(), v.focused));
        let ui = self.session.ui_visible(now);
        let away = session.idle_ms >= AWAY_AFTER_MS || self.paused;
        if away && !gaming && !ui {
            return IDLE_TICK;
        }
        let changing = now.saturating_sub(self.last_change_at) < CALM_AFTER_MS;
        if ui || gaming || focused || session.fullscreen || changing {
            TICK
        } else {
            CALM_TICK
        }
    }

    /// 前台切换、用户改模式时立即重新调度并落到硬件，不等下一次采样。
    /// 新的前台程序已经在上一轮采样里（切换到已经开着的程序，最常见）时直接沿用上一轮的采样，
    /// 响应更快；刚打开的程序还不在里面，才重新取一次进程快照（约 5 毫秒），让它也立即拿到前台待遇。
    /// 这样紧接着的状态查询就是最终结果（包括实际生效的模式），界面不会先显示一个中间状态。
    pub fn reschedule_now(&mut self) {
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
        let snapshot = if known {
            view.snapshot
        } else {
            self.snapshotter.take(now).unwrap_or(view.snapshot)
        };
        let classes = self.classify(&snapshot, now);
        let outcome = self.schedule_if(true, &snapshot, &classes);
        let effective_mode = self.apply_modes(outcome.effective_mode, now);
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
        let own: Vec<Pid> = self.own_pids.iter().copied().collect();
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

    /// 取采集线程最近一次的读数（不等待），并告诉它需要的详细程度：界面显示着时全读；
    /// 玩游戏、有重点程序时不调用机型接口（可能让所有核心短暂停顿）；其余时候不访问显卡
    /// （在 Optimus 笔记本上读显卡会唤醒休眠中的独显）。
    fn read_sensors(&mut self, outcome: &ScheduleOutcome, now: Millis) -> HardwareReading {
        self.sensors.set_level(if self.session.ui_visible(now) {
            SensorLevel::Full
        } else if outcome.game.is_some() {
            SensorLevel::Gaming
        } else if outcome.focused {
            SensorLevel::Focused
        } else {
            SensorLevel::Basic
        });
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
        match (limited_changed, headroom.limit_pct) {
            (Some(true), Some(p)) => {
                log::info!("处理器正被限速（性能上限 {p:.0}%），后台更早让路")
            }
            (Some(false), _) => log::info!("处理器不再被限速"),
            _ => {}
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
