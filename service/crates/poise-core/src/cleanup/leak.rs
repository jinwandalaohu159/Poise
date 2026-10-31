//! 泄漏检测：识别"只涨不跌"的进程。
//!
//! 只看趋势，不看绝对值：游戏、浏览器占用高但会上下波动，不算泄漏；
//! 持续数小时单调上涨、没有平台期的才算。

use crate::fast_hash::HashMap;
use crate::model::{Millis, ProcessKey, Snapshot};

use super::trend::{GrowthRule, sustained_growth};

#[derive(Debug, Clone, Copy)]
pub struct LeakConfig {
    /// 两个保存点之间的最小间隔（降采样）。
    pub sample_interval_ms: Millis,
    /// 最多保存多少个点（默认约 42 小时；满了之后早先的点隔一个丢一个，历史更长、更稀）。
    pub max_points: usize,
    /// 进程启动后多久开始统计（跳过启动阶段的正常增长）。
    pub warmup_ms: Millis,
    /// 至少观察多久才下结论。
    pub min_span_ms: Millis,
    pub min_points: usize,
    /// 拟合优度下限：越接近 1 越"单调"。
    pub min_r2: f64,
    pub memory: MetricThreshold,
    pub handles: MetricThreshold,
}

/// 单个指标的判定阈值。
#[derive(Debug, Clone, Copy)]
pub struct MetricThreshold {
    /// 每小时至少增长多少。
    pub min_slope_per_hour: f64,
    /// 观察期内至少增长多少（绝对值）。
    pub min_growth: f64,
    /// 观察期内至少增长多少（相对起点的比例）。
    pub min_growth_ratio: f64,
}

impl Default for LeakConfig {
    fn default() -> Self {
        Self {
            sample_interval_ms: 5 * 60 * 1000,
            max_points: 512,
            warmup_ms: 30 * 60 * 1000,
            min_span_ms: 60 * 60 * 1000,
            min_points: 8,
            min_r2: 0.7,
            memory: MetricThreshold {
                min_slope_per_hour: 20.0 * 1024.0 * 1024.0,
                min_growth: 64.0 * 1024.0 * 1024.0,
                min_growth_ratio: 0.5,
            },
            handles: MetricThreshold {
                min_slope_per_hour: 300.0,
                min_growth: 2000.0,
                min_growth_ratio: 0.5,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakMetric {
    Memory,
    Handles,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeakSuspect {
    pub key: ProcessKey,
    pub metric: LeakMetric,
    pub per_hour: f64,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    at: Millis,
    memory: f64,
    handles: f64,
}

#[derive(Debug, Default)]
struct Series {
    points: Vec<Point>,
}

#[derive(Debug, Default)]
pub struct LeakDetector {
    cfg: LeakConfig,
    series: HashMap<ProcessKey, Series>,
    /// 进程第一次被看到的时间，用于判断预热期。
    first_seen: HashMap<ProcessKey, Millis>,
    /// 上次看所有进程的时间（见 [`LeakDetector::OBSERVE_EVERY_MS`]）。
    observed_at: Option<Millis>,
}

impl LeakDetector {
    pub fn new(cfg: LeakConfig) -> Self {
        Self {
            cfg,
            ..Default::default()
        }
    }

    /// 多久看一次所有进程：泄漏按小时判断，每个进程每隔 `sample_interval_ms` 才取一个点，
    /// 不必每轮采样都把几百个进程过一遍。
    const OBSERVE_EVERY_MS: Millis = 60_000;

    pub fn observe(&mut self, snap: &Snapshot) {
        let now = snap.at;
        if self
            .observed_at
            .is_some_and(|t| now.saturating_sub(t) < Self::OBSERVE_EVERY_MS)
        {
            return;
        }
        self.observed_at = Some(now);
        for p in &snap.processes {
            let key = p.facts.key;
            let first = *self.first_seen.entry(key).or_insert(now);
            if now.saturating_sub(first) < self.cfg.warmup_ms {
                continue;
            }
            let series = self.series.entry(key).or_default();
            let due = series
                .points
                .last()
                .is_none_or(|last| now.saturating_sub(last.at) >= self.cfg.sample_interval_ms);
            if due {
                series.points.push(Point {
                    at: now,
                    memory: p.metrics.private_bytes as f64,
                    handles: p.metrics.handles as f64,
                });
                if series.points.len() > self.cfg.max_points {
                    // 满了就隔一个丢一个：保留更长的历史，只是早先的点稀一些（每满一次才挪一遍）。
                    let mut keep = false;
                    series.points.retain(|_| {
                        keep = !keep;
                        keep
                    });
                }
            }
        }
        let alive: std::collections::HashSet<_> =
            snap.processes.iter().map(|p| p.facts.key).collect();
        self.series.retain(|k, _| alive.contains(k));
        self.first_seen.retain(|k, _| alive.contains(k));
    }

    pub fn suspects(&self) -> Vec<LeakSuspect> {
        let mut out = Vec::new();
        for (key, s) in &self.series {
            for (metric, threshold, pick) in [
                (
                    LeakMetric::Memory,
                    &self.cfg.memory,
                    (|p: &Point| p.memory) as fn(&Point) -> f64,
                ),
                (LeakMetric::Handles, &self.cfg.handles, |p: &Point| {
                    p.handles
                }),
            ] {
                if let Some(s) = self.judge(*key, &s.points, metric, threshold, pick) {
                    out.push(s);
                }
            }
        }
        out.sort_by(|a, b| b.per_hour.total_cmp(&a.per_hour));
        out
    }

    fn judge(
        &self,
        key: ProcessKey,
        points: &[Point],
        metric: LeakMetric,
        t: &MetricThreshold,
        pick: fn(&Point) -> f64,
    ) -> Option<LeakSuspect> {
        let rule = GrowthRule {
            min_span_ms: self.cfg.min_span_ms,
            min_points: self.cfg.min_points,
            min_r2: self.cfg.min_r2,
            min_slope_per_hour: t.min_slope_per_hour,
            min_growth: t.min_growth,
            min_growth_ratio: t.min_growth_ratio,
        };
        let g = sustained_growth(points, |p| p.at, pick, &rule)?;
        Some(LeakSuspect {
            key,
            metric,
            per_hour: g.per_hour,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessMetrics, ProcessSample};

    const MB: u64 = 1024 * 1024;
    const MIN: Millis = 60 * 1000;
    const KEY: ProcessKey = ProcessKey { pid: 9, created: 1 };

    fn snap(at: Millis, private_mb: u64, handles: u32) -> Snapshot {
        Snapshot {
            at,
            processes: vec![ProcessSample {
                facts: ProcessFacts {
                    key: KEY,
                    ..Default::default()
                },
                metrics: ProcessMetrics {
                    private_bytes: private_mb * MB,
                    handles,
                    ..Default::default()
                },
            }],
            ..Default::default()
        }
    }

    /// 模拟运行 `hours` 小时，每 5 分钟采样一次。
    fn feed(d: &mut LeakDetector, hours: u64, f: impl Fn(u64) -> (u64, u32)) {
        for i in 0..=(hours * 12) {
            let at = i * 5 * MIN;
            let (mb, h) = f(at);
            d.observe(&snap(at, mb, h));
        }
    }

    #[test]
    fn steady_growth_is_a_memory_leak() {
        let mut d = LeakDetector::new(LeakConfig::default());
        // 每小时涨 100MB，像 audiodg 那样。
        feed(&mut d, 6, |at| (20 + at * 100 / (60 * MIN), 500));
        let s = d.suspects();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].metric, LeakMetric::Memory);
        assert!(s[0].per_hour > 90.0 * MB as f64);
    }

    #[test]
    fn high_but_fluctuating_usage_is_not_a_leak() {
        let mut d = LeakDetector::new(LeakConfig::default());
        // 游戏：占用 3GB 上下波动。
        feed(&mut d, 6, |at| {
            (
                3000 + if (at / (5 * MIN)).is_multiple_of(2) {
                    400
                } else {
                    0
                },
                2000,
            )
        });
        assert!(d.suspects().is_empty());
    }

    #[test]
    fn growth_that_plateaus_is_not_a_leak() {
        let mut d = LeakDetector::new(LeakConfig::default());
        // 前 2 小时涨，之后稳定（缓存填满）。
        feed(&mut d, 8, |at| {
            (200 + (at.min(150 * MIN)) * 200 / (60 * MIN), 800)
        });
        assert!(d.suspects().is_empty());
    }

    #[test]
    fn startup_growth_is_ignored_during_warmup() {
        let mut d = LeakDetector::new(LeakConfig::default());
        feed(&mut d, 1, |at| (100 + at * 500 / (60 * MIN), 100));
        assert!(d.suspects().is_empty());
    }

    #[test]
    fn handle_leak_is_detected() {
        let mut d = LeakDetector::new(LeakConfig::default());
        feed(&mut d, 6, |at| (50, 1000 + (at * 1000 / (60 * MIN)) as u32));
        let s = d.suspects();
        assert!(s.iter().any(|x| x.metric == LeakMetric::Handles));
    }
}
