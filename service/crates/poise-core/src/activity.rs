//! 把累计计数（CPU 时间、IO 字节）换算成速率，并记住每个进程最近一次"在干活"的时间。

use std::collections::HashMap;

use crate::model::{Millis, ProcessKey, Snapshot};

/// 一个进程最近的资源使用情况。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    /// 占用单个核心的百分比（满载一个核心 = 100）。
    pub core_pct: f32,
    /// IO 速率（字节/秒）。
    pub io_rate: f64,
    /// 最近一次超过活跃阈值的时间。
    pub last_active_at: Option<Millis>,
}

impl Usage {
    /// 占整机的百分比（单核百分比除以逻辑处理器数）。
    pub fn share_pct(&self, logical_cpus: u32) -> f32 {
        self.core_pct / logical_cpus.max(1) as f32
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActivityConfig {
    /// 超过这个单核百分比视为"在干活"。
    pub active_core_pct: f32,
    /// 超过这个 IO 速率视为"在干活"。
    pub active_io_rate: f64,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self {
            active_core_pct: 2.0,
            active_io_rate: 64.0 * 1024.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Counters {
    at: Millis,
    cpu_time: u64,
    io_bytes: u64,
}

#[derive(Debug, Default)]
pub struct ActivityTracker {
    cfg: ActivityConfig,
    last: HashMap<ProcessKey, Counters>,
    usage: HashMap<ProcessKey, Usage>,
    last_system: Option<(u64, u64)>,
    system_cpu_pct: f32,
}

impl ActivityTracker {
    pub fn new(cfg: ActivityConfig) -> Self {
        Self {
            cfg,
            ..Default::default()
        }
    }

    /// 整机 CPU 占用百分比（0–100）。
    pub fn system_cpu_pct(&self) -> f32 {
        self.system_cpu_pct
    }

    pub fn usage(&self, key: &ProcessKey) -> Usage {
        self.usage.get(key).copied().unwrap_or_default()
    }

    pub fn is_recently_active(&self, key: &ProcessKey, now: Millis, window_ms: Millis) -> bool {
        self.usage(key)
            .last_active_at
            .is_some_and(|t| now.saturating_sub(t) <= window_ms)
    }

    pub fn update(&mut self, snap: &Snapshot) {
        self.update_system(snap);

        let mut next = HashMap::with_capacity(snap.processes.len());
        for p in &snap.processes {
            let key = p.facts.key;
            let now = Counters {
                at: snap.at,
                cpu_time: p.metrics.cpu_time,
                io_bytes: p.metrics.io_bytes,
            };
            let prev_usage = self.usage.get(&key).copied().unwrap_or_default();
            let usage = match self.last.get(&key) {
                Some(prev) if now.at > prev.at => {
                    let secs = (now.at - prev.at) as f64 / 1000.0;
                    let cpu_secs = now.cpu_time.saturating_sub(prev.cpu_time) as f64 / 1e7;
                    let core_pct = (cpu_secs / secs * 100.0) as f32;
                    let io_rate = now.io_bytes.saturating_sub(prev.io_bytes) as f64 / secs;
                    let active =
                        core_pct >= self.cfg.active_core_pct || io_rate >= self.cfg.active_io_rate;
                    Usage {
                        core_pct,
                        io_rate,
                        last_active_at: if active {
                            Some(snap.at)
                        } else {
                            prev_usage.last_active_at
                        },
                    }
                }
                _ => prev_usage,
            };
            next.insert(key, now);
            self.usage.insert(key, usage);
        }
        // 已退出的进程不再保留。
        self.usage.retain(|k, _| next.contains_key(k));
        self.last = next;
    }

    fn update_system(&mut self, snap: &Snapshot) {
        let busy = snap.system.cpu_busy_time;
        let total = snap.system.cpu_total_time;
        if let Some((pb, pt)) = self.last_system
            && total > pt
        {
            self.system_cpu_pct =
                (busy.saturating_sub(pb) as f64 * 100.0 / (total - pt) as f64) as f32;
        }
        self.last_system = Some((busy, total));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessMetrics, ProcessSample};

    fn snap(at: Millis, cpu_time: u64, io: u64) -> Snapshot {
        Snapshot {
            at,
            processes: vec![ProcessSample {
                facts: ProcessFacts {
                    key: ProcessKey { pid: 7, created: 1 },
                    ..Default::default()
                },
                metrics: ProcessMetrics {
                    cpu_time,
                    io_bytes: io,
                    ..Default::default()
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn computes_core_percent_and_activity() {
        let mut t = ActivityTracker::new(ActivityConfig::default());
        let key = ProcessKey { pid: 7, created: 1 };
        t.update(&snap(0, 0, 0));
        // 2 秒内用了 1 秒 CPU → 50% 单核。
        t.update(&snap(2000, 10_000_000, 0));
        let u = t.usage(&key);
        assert!((u.core_pct - 50.0).abs() < 0.01);
        assert_eq!(u.last_active_at, Some(2000));
        // 之后空闲，最近活跃时间保持不变。
        t.update(&snap(4000, 10_000_000, 0));
        assert_eq!(t.usage(&key).last_active_at, Some(2000));
        assert!(t.is_recently_active(&key, 4000, 5000));
        assert!(!t.is_recently_active(&key, 60_000, 5000));
    }

    #[test]
    fn io_counts_as_activity() {
        let mut t = ActivityTracker::new(ActivityConfig::default());
        t.update(&snap(0, 0, 0));
        t.update(&snap(1000, 0, 10 * 1024 * 1024));
        let u = t.usage(&ProcessKey { pid: 7, created: 1 });
        assert!(u.last_active_at.is_some());
    }

    #[test]
    fn exited_processes_are_forgotten() {
        let mut t = ActivityTracker::new(ActivityConfig::default());
        t.update(&snap(0, 0, 0));
        t.update(&Snapshot {
            at: 1000,
            ..Default::default()
        });
        assert_eq!(
            t.usage(&ProcessKey { pid: 7, created: 1 }),
            Usage::default()
        );
    }
}
