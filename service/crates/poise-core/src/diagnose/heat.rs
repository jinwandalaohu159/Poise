//! 温度判断：用平滑后的温度，并且进入和退出"温度过高"用不同的阈值。
//! 短暂的尖峰不会触发；温度在阈值附近来回时，状态也不会反复切换。
//! 界面显示的温度仍是实时读数，这里只决定状态。

use super::{DiagnoseConfig, Thermals};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Heat {
    #[default]
    Normal,
    Hot,
    Critical,
}

#[derive(Debug, Default)]
pub struct HeatWatch {
    /// 平滑后的最高温度（CPU、显卡取高者）。
    smoothed: Option<f32>,
    last_ms: Option<u64>,
    hot: bool,
}

impl HeatWatch {
    /// 每次读到温度时调用，返回当前的温度状态。
    pub fn observe(&mut self, cfg: &DiagnoseConfig, thermals: Thermals, now_ms: u64) -> Heat {
        let Some(current) = [thermals.cpu_temp, thermals.gpu_temp]
            .into_iter()
            .flatten()
            .reduce(f32::max)
        else {
            // 读不到温度：不做判断，下次从头开始。
            *self = Self::default();
            return Heat::Normal;
        };
        let smoothed = match (self.smoothed, self.last_ms) {
            (Some(prev), Some(last)) => {
                let dt = now_ms.saturating_sub(last) as f32;
                let alpha = 1.0 - (-dt / cfg.heat_smoothing_ms as f32).exp();
                prev + (current - prev) * alpha
            }
            _ => current,
        };
        self.smoothed = Some(smoothed);
        self.last_ms = Some(now_ms);

        let threshold = if self.hot {
            cfg.hot_release_temp
        } else {
            cfg.hot_temp
        };
        self.hot = smoothed >= threshold;
        if smoothed >= cfg.critical_temp {
            Heat::Critical
        } else if self.hot {
            Heat::Hot
        } else {
            Heat::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP_MS: u64 = 2_000;

    fn cpu(t: f32) -> Thermals {
        Thermals {
            cpu_temp: Some(t),
            gpu_temp: None,
        }
    }

    /// 按固定间隔喂一串温度，返回每次的状态。
    fn run(watch: &mut HeatWatch, start_ms: u64, temps: &[f32]) -> Vec<Heat> {
        let cfg = DiagnoseConfig::default();
        temps
            .iter()
            .enumerate()
            .map(|(i, &t)| watch.observe(&cfg, cpu(t), start_ms + i as u64 * STEP_MS))
            .collect()
    }

    #[test]
    fn a_short_spike_does_not_trigger() {
        let mut w = HeatWatch::default();
        run(&mut w, 0, &[80.0; 20]);
        let states = run(&mut w, 40_000, &[94.0, 80.0, 80.0]);
        assert!(states.iter().all(|&s| s == Heat::Normal), "{states:?}");
    }

    #[test]
    fn hovering_around_the_threshold_does_not_flicker() {
        let mut w = HeatWatch::default();
        // 持续高温后进入"温度过高"。
        let entering = run(&mut w, 0, &[93.0; 30]);
        assert_eq!(entering.last(), Some(&Heat::Hot));
        // 在 86～89 之间来回：高于退出阈值，一直保持，不来回切换。
        let hovering: Vec<f32> = (0..30)
            .map(|i| if i % 2 == 0 { 86.0 } else { 89.0 })
            .collect();
        let states = run(&mut w, 60_000, &hovering);
        assert!(states.iter().all(|&s| s == Heat::Hot), "{states:?}");
        // 降下来后退出。
        let cooled = run(&mut w, 120_000, &[78.0; 30]);
        assert_eq!(cooled.last(), Some(&Heat::Normal));
    }

    #[test]
    fn values_just_below_the_entry_threshold_stay_normal() {
        let mut w = HeatWatch::default();
        let states = run(&mut w, 0, &[88.0; 60]);
        assert!(states.iter().all(|&s| s == Heat::Normal));
    }

    #[test]
    fn sustained_extreme_heat_is_critical() {
        let mut w = HeatWatch::default();
        let states = run(&mut w, 0, &[99.0; 30]);
        assert_eq!(states.last(), Some(&Heat::Critical));
    }

    #[test]
    fn hottest_sensor_counts_and_missing_readings_reset() {
        let cfg = DiagnoseConfig::default();
        let mut w = HeatWatch::default();
        let both = Thermals {
            cpu_temp: Some(60.0),
            gpu_temp: Some(93.0),
        };
        assert_eq!(w.observe(&cfg, both, 0), Heat::Hot);
        assert_eq!(w.observe(&cfg, Thermals::default(), 2_000), Heat::Normal);
        // 重新开始：第一次读数直接作为平滑值。
        assert_eq!(w.observe(&cfg, cpu(70.0), 4_000), Heat::Normal);
    }
}
