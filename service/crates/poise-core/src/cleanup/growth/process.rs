//! 进程：每个进程运行稳定后的基线，以及明显超出基线的增长。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::GrowthConfig;
use crate::model::{Millis, ProcessKey, ProcessMetrics, Snapshot};

/// 增长的是哪一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthMetric {
    Handles,
    Threads,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    handles: u64,
    threads: u64,
    private_bytes: u64,
}

impl From<&ProcessMetrics> for Counts {
    fn from(m: &ProcessMetrics) -> Self {
        Self {
            handles: u64::from(m.handles),
            threads: u64::from(m.threads),
            private_bytes: m.private_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProcessBase {
    first_seen: Millis,
    /// 记录基线的时刻与当时的数值；稳定之前为 None。
    base: Option<(Millis, Counts)>,
}

/// 某一项从基线到现在的变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub metric: GrowthMetric,
    pub from: u64,
    pub to: u64,
}

/// 一个进程相对自己基线的异常增长。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grown {
    pub key: ProcessKey,
    pub change: Change,
}

/// 每个进程的基线。
#[derive(Debug, Default)]
pub(super) struct ProcessGrowth {
    processes: HashMap<ProcessKey, ProcessBase>,
    /// 上次看所有进程的时间（见 [`ProcessGrowth::OBSERVE_EVERY_MS`]）。
    observed_at: Option<Millis>,
}

impl ProcessGrowth {
    /// 多久看一次所有进程：基线在进程启动几分钟后记一次、增长按半小时以上判断，不必每轮都看。
    const OBSERVE_EVERY_MS: Millis = 30_000;

    /// 忘掉退出的进程；新进程从看到它算起，超过 `settle_ms` 后记录基线。
    pub(super) fn observe(&mut self, snap: &Snapshot, now: Millis, settle_ms: Millis) {
        if self
            .observed_at
            .is_some_and(|t| now.saturating_sub(t) < Self::OBSERVE_EVERY_MS)
        {
            return;
        }
        self.observed_at = Some(now);
        let alive: HashSet<ProcessKey> = snap.processes.iter().map(|p| p.facts.key).collect();
        self.processes.retain(|k, _| alive.contains(k));
        for p in &snap.processes {
            let entry = self.processes.entry(p.facts.key).or_insert(ProcessBase {
                first_seen: now,
                base: None,
            });
            if entry.base.is_none() && now.saturating_sub(entry.first_seen) >= settle_ms {
                entry.base = Some((now, Counts::from(&p.metrics)));
            }
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    pub(super) fn grown(&self, snap: &Snapshot, now: Millis, cfg: &GrowthConfig) -> Vec<Grown> {
        snap.processes
            .iter()
            .filter_map(|p| {
                let (at, base) = self.processes.get(&p.facts.key)?.base?;
                if now.saturating_sub(at) < cfg.min_observed_ms {
                    return None;
                }
                let current = Counts::from(&p.metrics);
                [
                    (
                        GrowthMetric::Handles,
                        cfg.handles,
                        base.handles,
                        current.handles,
                    ),
                    (
                        GrowthMetric::Threads,
                        cfg.threads,
                        base.threads,
                        current.threads,
                    ),
                    (
                        GrowthMetric::Memory,
                        cfg.memory,
                        base.private_bytes,
                        current.private_bytes,
                    ),
                ]
                .into_iter()
                .find(|(_, limit, from, to)| limit.exceeded(*from, *to))
                .map(|(metric, _, from, to)| Grown {
                    key: p.facts.key,
                    change: Change { metric, from, to },
                })
            })
            .collect()
    }
}
