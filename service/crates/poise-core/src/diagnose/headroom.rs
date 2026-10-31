//! 处理器还有没有余量：处理器自己报告正被限速（温度墙、功耗墙），或者温度到了临界。
//! 没有余量时，后台更早让路，把有限的功耗和散热留给正在用的程序。
//!
//! 限速信号直接来自处理器，各家处理器通用。温度只在读不到限速信号时作为替代（见 [`HeatWatch`]）：
//! 不同处理器的温度上限不同，笔记本处理器在高性能模式下本来就工作在 90～100°C（锐龙到 95°C 才开始
//! 提速受限），温度高不等于没有余量——实测开机后一两分钟温度超过 95°C、却从没被限速，
//! 后台程序因此被完整让路一两分钟又恢复。

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
    /// 没有余量了：正被限速；温度到了临界；读不到限速信号时，温度贴着上限。
    pub fn exhausted(&self) -> bool {
        self.limited
            || self.heat == Heat::Critical
            || (self.heat == Heat::Hot && self.limit_pct.is_none())
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

    /// 最近一次是否正被固件限速。
    pub fn limited(&self) -> bool {
        self.last.limited
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
    fn a_hot_cpu_with_headroom_left_is_not_exhausted() {
        let cfg = DiagnoseConfig::default();
        let hot = Thermals {
            cpu_temp: Some(cfg.hot_temp + 1.0),
            gpu_temp: None,
        };
        // 处理器说没有限速：温度高也还有余量。
        let mut w = HeadroomWatch::default();
        let (h, _) = w.observe(&cfg, hot, &system(Some(100.0)), 0);
        assert_eq!(h.heat, Heat::Hot);
        assert!(!h.exhausted());
        // 读不到限速信号：只能按温度。
        let mut w = HeadroomWatch::default();
        assert!(w.observe(&cfg, hot, &system(None), 0).0.exhausted());
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
