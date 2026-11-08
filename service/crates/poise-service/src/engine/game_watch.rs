//! 玩游戏时处理器、显卡实际怎样：每分钟在日志里写一行，游戏结束时写一段总结。帧数上不去时看它就知道是
//! 大核被功耗、温度压低了频率（`poise_core::diagnose::ClockWatch`），还是显卡跑满了，还是都不是。
//!
//! 只用已经在读的数据：各核心的实际频率（处理器性能计数器）、各核心的占用、显卡驱动的占用和温度。处理器
//! 温度玩游戏时不读（机型接口每次调用都可能让所有核心短暂停顿，见 `poise_hardware::SensorLevel`）。

use poise_core::diagnose::{ClassClocks, class_clocks};
use poise_core::model::{Millis, Snapshot};
use poise_core::prefs::Mode;
use poise_core::schedule::GameInfo;
use poise_hardware::HardwareReading;

use super::Engine;

/// 玩游戏时多久写一行。
const LOG_EVERY_MS: Millis = 60_000;
/// 两轮之间最多按这么久累计（服务卡住、睡眠时不把空白算进去）。
const MAX_STEP_MS: Millis = 10_000;

/// 正在玩的一局。
#[derive(Debug)]
pub(super) struct Stint {
    name: String,
    started_at: Millis,
    last_at: Millis,
    logged_at: Millis,
    /// 大核频率：按时间累计的总和（MHz·毫秒）、累计时长、最低。
    mhz_ms: f64,
    mhz_time: Millis,
    mhz_min: Option<u32>,
    /// 大核被功耗或温度压着的累计时长。
    held_ms: Millis,
    gpu_sum: f64,
    gpu_samples: u32,
    gpu_peak: f32,
    gpu_temp_peak: Option<f32>,
}

/// 游戏期间的观察（见模块说明）。
#[derive(Debug, Default)]
pub(super) struct GameWatch {
    stint: Option<Stint>,
}

impl Engine {
    /// 每轮调用：有游戏时记一轮、到时间写一行；游戏结束时写总结。
    pub(super) fn watch_game(
        &mut self,
        snapshot: &Snapshot,
        reading: &HardwareReading,
        game: Option<&GameInfo>,
        mode: Mode,
        now: Millis,
    ) {
        let clocks = class_clocks(
            &snapshot.core_mhz,
            self.activity.processor_loads(),
            &self.topology.performance_by_index,
        );
        let held = self.clocks.observe(&clocks);
        let Some(game) = game else {
            if let Some(stint) = self.game_watch.stint.take() {
                log::info!("{}", self.summary(&stint, now));
            }
            return;
        };
        let stint = self
            .game_watch
            .stint
            .get_or_insert_with(|| Stint::start(&game.name, now));
        stint.add(&clocks, held, reading, now);
        if now.saturating_sub(stint.logged_at) < LOG_EVERY_MS {
            return;
        }
        stint.logged_at = now;
        let load = snapshot
            .find(game.pid)
            .map(|p| self.activity.usage(&p.facts.key).core_pct);
        let line = self.minute_line(game, &clocks, held, load, reading, mode);
        log::info!("{line}");
    }

    /// 每分钟的一行：大核、小核频率，整机和游戏的占用，显卡，模式。
    fn minute_line(
        &self,
        game: &GameInfo,
        clocks: &ClassClocks,
        held: bool,
        game_load: Option<f32>,
        reading: &HardwareReading,
        mode: Mode,
    ) -> String {
        let mut parts = vec![format!(
            "大核 {}（{} 个线程在忙，最高见过 {}）",
            ghz(clocks.performance_mhz),
            clocks.performance_busy,
            ghz(self.clocks.peak_mhz())
        )];
        if self.topology.efficiency_count.cores > 0 {
            parts.push(format!("小核 {}", ghz(clocks.efficiency_mhz)));
        }
        parts.push(format!("整机 {:.0}%", self.activity.system_cpu_pct()));
        if let Some(load) = game_load {
            parts.push(format!("游戏 {load:.0}%（100% = 一个核心）"));
        }
        parts.push(gpu(reading));
        parts.push(format!("模式 {mode:?}"));
        let verdict = if held {
            "：大核在忙、频率却不到同样多的大核在忙时跑到过的八成，正被功耗或温度压着"
        } else {
            ""
        };
        format!("游戏中（{}）：{}{verdict}", game.name, parts.join("，"))
    }

    /// 游戏结束时的总结。
    fn summary(&self, s: &Stint, now: Millis) -> String {
        let minutes = now.saturating_sub(s.started_at) / 60_000;
        let avg = (s.mhz_time > 0).then(|| (s.mhz_ms / s.mhz_time as f64).round() as u32);
        let gpu = if s.gpu_samples > 0 {
            let temp = s
                .gpu_temp_peak
                .map_or(String::new(), |t| format!("、最高 {t:.0}°C"));
            format!(
                "显卡占用平均 {:.0}%、最高 {:.0}%{temp}",
                s.gpu_sum / f64::from(s.gpu_samples),
                s.gpu_peak
            )
        } else {
            "显卡读不到".into()
        };
        format!(
            "游戏结束（{}，{minutes} 分钟）：大核平均 {}、最低 {}（最高见过 {}），被功耗或温度压着 {} 分钟；{gpu}",
            s.name,
            ghz(avg),
            ghz(s.mhz_min),
            ghz(self.clocks.peak_mhz()),
            s.held_ms / 60_000
        )
    }
}

impl Stint {
    fn start(name: &str, now: Millis) -> Self {
        Self {
            name: name.to_string(),
            started_at: now,
            last_at: now,
            logged_at: now,
            mhz_ms: 0.0,
            mhz_time: 0,
            mhz_min: None,
            held_ms: 0,
            gpu_sum: 0.0,
            gpu_samples: 0,
            gpu_peak: 0.0,
            gpu_temp_peak: None,
        }
    }

    fn add(&mut self, clocks: &ClassClocks, held: bool, reading: &HardwareReading, now: Millis) {
        let step = now.saturating_sub(self.last_at).min(MAX_STEP_MS);
        self.last_at = now;
        if let Some(mhz) = clocks.performance_mhz
            && clocks.performance_busy > 0
        {
            self.mhz_ms += f64::from(mhz) * step as f64;
            self.mhz_time += step;
            self.mhz_min = Some(self.mhz_min.map_or(mhz, |m| m.min(mhz)));
        }
        if held {
            self.held_ms += step;
        }
        if let Some(pct) = reading.gpu_pct {
            self.gpu_sum += f64::from(pct);
            self.gpu_samples += 1;
            self.gpu_peak = self.gpu_peak.max(pct);
        }
        if let Some(t) = reading.thermals.gpu_temp {
            self.gpu_temp_peak = Some(self.gpu_temp_peak.map_or(t, |p| p.max(t)));
        }
    }
}

/// "3.4 GHz"；读不到时"读不到"。
fn ghz(mhz: Option<u32>) -> String {
    mhz.map_or_else(
        || "读不到".into(),
        |m| format!("{:.1} GHz", f64::from(m) / 1000.0),
    )
}

/// 显卡占用与温度。
fn gpu(reading: &HardwareReading) -> String {
    match (reading.gpu_pct, reading.thermals.gpu_temp) {
        (Some(p), Some(t)) => format!("显卡 {p:.0}%、{t:.0}°C"),
        (Some(p), None) => format!("显卡 {p:.0}%"),
        (None, Some(t)) => format!("显卡 {t:.0}°C"),
        (None, None) => "显卡读不到".into(),
    }
}
