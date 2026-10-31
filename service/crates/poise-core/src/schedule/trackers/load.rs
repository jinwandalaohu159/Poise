//! 程序的负载：整棵进程树的处理器占用与磁盘读写之和，按时间平滑。
//!
//! 一个程序的工作常分散在多个进程里（浏览器的各个子进程、编译时不断新建又退出的编译器进程），
//! 按单个进程看会漏掉；按整棵树加起来才是这个程序真正的负载。
//! 平滑后，时忙时闲的工作（编译、下载）按平均负载判断，几秒钟的尖峰影响很小。

use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::fast_hash::HashMap;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};

/// 一个程序的负载。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TreeUsage {
    /// 处理器（单核百分比）。
    pub core_pct: f32,
    /// 磁盘读写（字节每秒）。
    pub disk_bytes_per_s: f64,
}

/// 一个程序平滑后的负载：慢的（判断在不在忙、核心范围收不收回，短暂的尖峰影响很小）和快的
/// （判断是不是重点程序，几秒内跟上）。
#[derive(Debug, Clone, Copy, Default)]
struct Smoothed {
    slow: TreeUsage,
    fast: f32,
    at: Millis,
}

/// 平滑的时间常数。
#[derive(Debug, Clone, Copy)]
pub struct LoadSmoothing {
    /// 慢的：判断后台在不在忙。
    pub slow_ms: Millis,
    /// 快的：判断正在用的程序是不是在持续干重活。
    pub fast_ms: Millis,
}

#[derive(Debug, Default)]
pub struct TreeLoad {
    /// 各进程树（按根进程）平滑后的负载。
    smoothed: HashMap<ProcessKey, Smoothed>,
    /// 根进程号 → 根进程（进程号会被复用，按进程身份记）。
    roots: HashMap<Pid, ProcessKey>,
    /// 各进程树此刻的处理器占用（不平滑，单核百分比）：负载突然升高时立即可见。
    instant: HashMap<ProcessKey, f32>,
    /// 上一轮的。
    previous: HashMap<ProcessKey, f32>,
    /// 上一次汇总的采样时刻。
    sampled_at: Option<Millis>,
}

impl TreeLoad {
    /// 每轮调用一次：汇总各进程树此刻的占用，更新平滑值。
    pub fn update(
        &mut self,
        snap: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        activity: &ActivityTracker,
        now: Millis,
        smoothing: LoadSmoothing,
    ) {
        let by_pid: HashMap<Pid, ProcessKey> = snap
            .processes
            .iter()
            .map(|p| (p.facts.key.pid, p.facts.key))
            .collect();
        let mut current: HashMap<ProcessKey, TreeUsage> = HashMap::default();
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
        }
        self.smoothed.retain(|k, _| current.contains_key(k));
        let instant = current.iter().map(|(k, u)| (*k, u.core_pct)).collect();
        // 立即重新调度沿用上一轮的采样（前台切换时）：同一份采样不算"又一轮"，否则一次尖峰会被当成
        // 连续两轮都达到（见 [`TreeLoad::sustained`]）。
        if self.sampled_at.replace(snap.at) == Some(snap.at) {
            self.instant = instant;
        } else {
            self.previous = std::mem::replace(&mut self.instant, instant);
        }
        for (root, load) in current {
            // 新出现的程序从 0 开始：刚启动时的短暂高负载不会直接算在忙。
            let entry = self.smoothed.entry(root).or_insert(Smoothed {
                at: now,
                ..Smoothed::default()
            });
            let dt = now.saturating_sub(entry.at);
            entry.slow = smooth(entry.slow, load, dt, smoothing.slow_ms);
            entry.fast += (load.core_pct - entry.fast) * alpha(dt, smoothing.fast_ms);
            entry.at = now;
        }
        self.roots = self.smoothed.keys().map(|k| (k.pid, *k)).collect();
    }

    /// 进程树的根进程。
    pub fn root(&self, root_pid: Pid) -> Option<ProcessKey> {
        self.roots.get(&root_pid).copied()
    }

    /// 进程树平滑后的负载（慢的）。
    pub fn of(&self, root: &ProcessKey) -> TreeUsage {
        self.smoothed.get(root).map(|s| s.slow).unwrap_or_default()
    }

    /// 进程树平滑后的处理器占用（快的，单核百分比）。
    pub fn recent(&self, root: &ProcessKey) -> f32 {
        self.smoothed.get(root).map_or(0.0, |s| s.fast)
    }

    /// 进程树此刻的处理器占用（不平滑，单核百分比）。
    pub fn instant(&self, root: &ProcessKey) -> f32 {
        self.instant.get(root).copied().unwrap_or_default()
    }

    /// 连续两轮都达到的处理器占用（单核百分比）：真的变忙了，下一轮就看得到；只有一轮的尖峰
    /// （网页加载、自动存档）不算。
    pub fn sustained(&self, root: &ProcessKey) -> f32 {
        let previous = self.previous.get(root).copied().unwrap_or_default();
        self.instant(root).min(previous)
    }
}

/// 指数平滑的系数：经过 `dt_ms`，时间常数 `tau_ms` 内的变化只体现约 63%。
fn alpha(dt_ms: Millis, tau_ms: Millis) -> f32 {
    (1.0 - (-(dt_ms as f64) / tau_ms.max(1) as f64).exp()) as f32
}

fn smooth(prev: TreeUsage, load: TreeUsage, dt_ms: Millis, tau_ms: Millis) -> TreeUsage {
    let a = alpha(dt_ms, tau_ms);
    TreeUsage {
        core_pct: prev.core_pct + (load.core_pct - prev.core_pct) * a,
        disk_bytes_per_s: prev.disk_bytes_per_s
            + (load.disk_bytes_per_s - prev.disk_bytes_per_s) * f64::from(a),
    }
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
        let mut v = TreeUsage::default();
        // 4 秒的尖峰：平滑值远低于尖峰。
        v = smooth(v, load, 2_000, 30_000);
        v = smooth(v, load, 2_000, 30_000);
        assert!(v.core_pct < 10.0 && v.disk_bytes_per_s < 10e6, "{v:?}");
        // 持续负载：平滑值逐渐接近。
        for _ in 3..40 {
            v = smooth(v, load, 2_000, 30_000);
        }
        assert!(v.core_pct > 50.0 && v.disk_bytes_per_s > 50e6);
    }

    #[test]
    fn the_fast_average_catches_up_within_seconds() {
        // 单线程跑满（100%）：快的 10 秒内超过 80%，慢的要将近一分钟。
        let mut fast = 0.0_f32;
        let mut slow = TreeUsage::default();
        let full = TreeUsage {
            core_pct: 100.0,
            disk_bytes_per_s: 0.0,
        };
        for _ in 0..5 {
            fast += (100.0 - fast) * alpha(2_000, 5_000);
            slow = smooth(slow, full, 2_000, 30_000);
        }
        assert!(fast > 80.0, "{fast}");
        assert!(slow.core_pct < 30.0);
    }
}
