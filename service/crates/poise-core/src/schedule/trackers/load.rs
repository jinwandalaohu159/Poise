//! 程序的负载：整棵进程树的处理器占用之和，按时间平滑。
//!
//! 一个程序的工作常分散在多个进程里（浏览器的各个子进程、编译时不断新建又退出的编译器进程），
//! 按单个进程看会漏掉；按整棵树加起来才是这个程序真正的负载。
//! 平滑后，时忙时闲的工作（编译、下载）按平均负载判断，几秒钟的尖峰影响很小。

use std::collections::HashMap;

use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};

#[derive(Debug, Default)]
pub struct TreeLoad {
    /// 各进程树（按根进程）平滑后的负载（单核百分比）与上次更新时间。
    smoothed: HashMap<ProcessKey, (f32, Millis)>,
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
        let mut current: HashMap<ProcessKey, f32> = HashMap::new();
        let mut foreground = 0.0;
        for p in &snap.processes {
            let Some(c) = classes.get(&p.facts.key.pid) else {
                continue;
            };
            let Some(&root) = by_pid.get(&c.root_pid) else {
                continue;
            };
            let load = activity.usage(&p.facts.key).core_pct;
            *current.entry(root).or_default() += load;
            if c.in_foreground_tree {
                foreground += load;
            }
        }
        self.smoothed.retain(|k, _| current.contains_key(k));
        for (root, load) in current {
            // 新出现的程序从 0 开始：刚启动时的短暂高负载不会直接算在忙。
            let entry = self.smoothed.entry(root).or_insert((0.0, now));
            *entry = smooth(*entry, load, now, smoothing_ms);
        }
        self.roots = self.smoothed.keys().map(|k| (k.pid, *k)).collect();
        self.foreground = foreground;
    }

    /// 进程树的根进程。
    pub fn root(&self, root_pid: Pid) -> Option<ProcessKey> {
        self.roots.get(&root_pid).copied()
    }

    /// 进程树平滑后的负载（单核百分比）。
    pub fn of(&self, root: &ProcessKey) -> f32 {
        self.smoothed.get(root).map_or(0.0, |(load, _)| *load)
    }

    /// 前台进程树此刻的负载（单核百分比）。
    pub fn foreground(&self) -> f32 {
        self.foreground
    }
}

/// 指数平滑：时间常数 `tau_ms` 内的变化只体现约 63%。
fn smooth((prev, at): (f32, Millis), load: f32, now: Millis, tau_ms: Millis) -> (f32, Millis) {
    let dt = now.saturating_sub(at) as f32;
    let alpha = 1.0 - (-dt / tau_ms.max(1) as f32).exp();
    (prev + (load - prev) * alpha, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_follows_sustained_load_but_damps_spikes() {
        let mut v = (0.0, 0);
        // 4 秒的尖峰：平滑值远低于尖峰。
        v = smooth(v, 60.0, 2_000, 30_000);
        v = smooth(v, 60.0, 4_000, 30_000);
        assert!(v.0 < 10.0, "{}", v.0);
        // 持续负载：平滑值逐渐接近。
        for i in 3..40 {
            v = smooth(v, 60.0, i * 2_000, 30_000);
        }
        assert!(v.0 > 50.0);
    }
}
