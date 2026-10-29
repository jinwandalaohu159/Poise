//! 处理器还有没有余量：温度贴着上限，或者处理器自己报告正被限速（温度墙、功耗墙）。
//! 没有余量时，后台更早让路，把有限的功耗和散热留给正在用的程序。
//!
//! 限速信号直接来自处理器，不依赖机型能不能读到温度；温度判断是它的补充（见 [`HeatWatch`]）。

use super::{DiagnoseConfig, Heat, HeatWatch, Thermals, is_limited};
use crate::model::SystemMetrics;

/// 此刻的余量。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Headroom {
    pub heat: Heat,
    /// 正被固件限速。
    pub limited: bool,
    /// 固件允许的性能上限（百分比），读不到时为空。
    pub limit_pct: Option<f32>,
}

impl Headroom {
    /// 没有余量了。
    pub fn exhausted(&self) -> bool {
        self.heat != Heat::Normal || self.limited
    }
}

#[derive(Debug, Default)]
pub struct HeadroomWatch {
    heat: HeatWatch,
    last: Headroom,
}

impl HeadroomWatch {
    /// 每轮调用一次。返回此刻的余量，以及"是否被限速"有没有变化（变化时为 Some，用于记日志）。
    pub fn observe(
        &mut self,
        cfg: &DiagnoseConfig,
        thermals: Thermals,
        system: &SystemMetrics,
        now_ms: u64,
    ) -> (Headroom, Option<bool>) {
        let now = Headroom {
            heat: self.heat.observe(cfg, thermals, now_ms),
            limited: is_limited(cfg, system),
            limit_pct: system.cpu_perf_limit_pct,
        };
        let changed = (now.limited != self.last.limited).then_some(now.limited);
        self.last = now;
        (now, changed)
    }

    /// 最近一次的结果。
    pub fn exhausted(&self) -> bool {
        self.last.exhausted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(limit: Option<f32>) -> SystemMetrics {
        SystemMetrics {
            cpu_perf_limit_pct: limit,
            ..Default::default()
        }
    }

    #[test]
    fn being_limited_exhausts_the_headroom_even_without_temperatures() {
        let cfg = DiagnoseConfig::default();
        let mut w = HeadroomWatch::default();
        let none = Thermals::default();
        let (h, changed) = w.observe(&cfg, none, &system(Some(100.0)), 0);
        assert!(!h.exhausted());
        assert_eq!(changed, None);

        let (h, changed) = w.observe(&cfg, none, &system(Some(70.0)), 2_000);
        assert!(h.exhausted() && w.exhausted());
        assert_eq!(changed, Some(true));
        // 持续限速：不再报告变化。
        assert_eq!(w.observe(&cfg, none, &system(Some(70.0)), 4_000).1, None);
        assert_eq!(w.observe(&cfg, none, &system(None), 6_000).1, Some(false));
        assert!(!w.exhausted());
    }

    #[test]
    fn a_hot_cpu_exhausts_the_headroom_too() {
        let cfg = DiagnoseConfig::default();
        let mut w = HeadroomWatch::default();
        let hot = Thermals {
            cpu_temp: Some(cfg.critical_temp + 1.0),
            gpu_temp: None,
        };
        let (h, changed) = w.observe(&cfg, hot, &system(None), 0);
        assert!(h.exhausted());
        // 只有限速的变化才报告（温度另有健康状态展示）。
        assert_eq!(changed, None);
    }
}
