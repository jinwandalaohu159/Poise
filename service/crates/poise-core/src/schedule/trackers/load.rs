//! 程序的负载：整棵进程树的处理器占用与磁盘读写之和，按时间平滑。
//!
//! 一个程序的工作常分散在多个进程里（浏览器的各个子进程、编译时不断新建又退出的编译器进程），
//! 按单个进程看会漏掉；按整棵树加起来才是这个程序真正的负载。
//! 平滑后，时忙时闲的工作（编译、下载）按平均负载判断，几秒钟的尖峰影响很小。

use std::collections::HashMap;

use super::busy::TreeUsage;
use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};

#[derive(Debug, Default)]
pub struct TreeLoad {
    /// 各进程树（按根进程）平滑后的负载与上次更新时间。
    smoothed: HashMap<ProcessKey, (TreeUsage, Millis)>,
    /// 根进程号 → 根进程（进程号会被复用，按进程身份记）。
    roots: HashMap<Pid, ProcessKey>,
    /// 前台进程树此刻的负载（不平滑）。
    foreground: f32,
}

impl TreeLoad {
    /// 每轮调用一次：汇总各进程树此刻的占用，更新平滑值。`smoothing_ms` 是平滑的时间常数。
    pub fn update(
        &mut self,
        snap: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        activity: &ActivityTracker,
        now: Millis,
        smoothing_ms: Millis,
    ) {
        let by_pid: HashMap<Pid, ProcessKey> = snap
            .processes
            .iter()
            .map(|p| (p.facts.key.pid, p.facts.key))
            .collect();
        let mut current: HashMap<ProcessKey, TreeUsage> = HashMap::new();
        let mut foreground = 0.0;
        for p in &snap.processes {
            let Some(c) = classes.get(&p.facts.key.pid) else {
                continue;
            };
            let Some(&root) = by_pid.get(&c.root_pid) else {
                continue;
            };
            let usage = activity.usage(&p.facts.key);
            let tree = current.entry(root).or_default();
            tree.core_pct += usage.core_pct;
            tree.disk_bytes_per_s += usage.disk_rate;
            if c.in_foreground_tree {
                foreground += usage.core_pct;
            }
        }
        self.smoothed.retain(|k, _| current.contains_key(k));
        for (root, load) in current {
            // 新出现的程序从 0 开始：刚启动时的短暂高负载不会直接算在忙。
            let entry = self
                .smoothed
                .entry(root)
                .or_insert((TreeUsage::default(), now));
            *entry = smooth(*entry, load, now, smoothing_ms);
        }
        self.roots = self.smoothed.keys().map(|k| (k.pid, *k)).collect();
        self.foreground = foreground;
    }

    /// 进程树的根进程。
    pub fn root(&self, root_pid: Pid) -> Option<ProcessKey> {
        self.roots.get(&root_pid).copied()
    }

    /// 进程树平滑后的负载。
    pub fn of(&self, root: &ProcessKey) -> TreeUsage {
        self.smoothed.get(root).map(|(u, _)| *u).unwrap_or_default()
    }

    /// 前台进程树此刻的负载（单核百分比）。
    pub fn foreground(&self) -> f32 {
        self.foreground
    }
}

/// 指数平滑：时间常数 `tau_ms` 内的变化只体现约 63%。
fn smooth(
    (prev, at): (TreeUsage, Millis),
    load: TreeUsage,
    now: Millis,
    tau_ms: Millis,
) -> (TreeUsage, Millis) {
    let dt = now.saturating_sub(at) as f64;
    let alpha = 1.0 - (-dt / tau_ms.max(1) as f64).exp();
    let usage = TreeUsage {
        core_pct: prev.core_pct + (load.core_pct - prev.core_pct) * alpha as f32,
        disk_bytes_per_s: prev.disk_bytes_per_s
            + (load.disk_bytes_per_s - prev.disk_bytes_per_s) * alpha,
    };
    (usage, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_follows_sustained_load_but_damps_spikes() {
        let load = TreeUsage {
            core_pct: 60.0,
            disk_bytes_per_s: 60e6,
        };
        let mut v = (TreeUsage::default(), 0);
        // 4 秒的尖峰：平滑值远低于尖峰。
        v = smooth(v, load, 2_000, 30_000);
        v = smooth(v, load, 4_000, 30_000);
        assert!(
            v.0.core_pct < 10.0 && v.0.disk_bytes_per_s < 10e6,
            "{:?}",
            v.0
        );
        // 持续负载：平滑值逐渐接近。
        for i in 3..40 {
            v = smooth(v, load, i * 2_000, 30_000);
        }
        assert!(v.0.core_pct > 50.0 && v.0.disk_bytes_per_s > 50e6);
    }
}
