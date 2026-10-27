//! 卡顿归因：刚才为什么卡。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::activity::ActivityTracker;
use crate::classify::{Classification, Role};
use crate::cleanup::LeakSuspect;
use crate::model::{Pid, Snapshot};

use super::{DiagnoseConfig, Thermals, is_throttling};

/// 占整机 CPU 低于这个百分比的程序不列出（列出来也说明不了问题）。
const MIN_LISTED_CPU_PCT: f32 = 0.5;

/// "刚才为什么卡"的结论。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StutterCause {
    /// 过热或功耗墙导致降频。
    Thermal,
    /// 后台程序在抢 CPU。
    BackgroundCpu { name: String, core_pct: f32 },
    /// 内存不够用了。
    MemoryPressure,
    /// 有程序越用越占资源。
    Leaks { count: usize },
    /// 前台程序本身就很重。
    ForegroundHeavy { name: String },
    /// 没发现明显问题（可能是资源管理器插件等）。
    Unclear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StutterReport {
    pub cause: StutterCause,
    pub cpu_pct: f32,
    pub mem_used_pct: f32,
    /// 占整机 CPU 的百分比（与任务管理器一致，不是单核百分比）。
    pub top_cpu: Vec<(String, f32)>,
    pub top_memory: Vec<(String, u64)>,
}

pub struct StutterInput<'a> {
    pub snapshot: &'a Snapshot,
    pub classes: &'a HashMap<Pid, Classification>,
    pub activity: &'a ActivityTracker,
    pub thermals: Thermals,
    pub suspects: &'a [LeakSuspect],
}

pub fn analyze_stutter(cfg: &DiagnoseConfig, input: &StutterInput) -> StutterReport {
    let snap = input.snapshot;
    let cpu_pct = input.activity.system_cpu_pct();
    let mut by_cpu: Vec<(&str, String, f32, bool)> = snap
        .processes
        .iter()
        .filter_map(|p| {
            let c = input.classes.get(&p.facts.key.pid)?;
            (c.role != Role::Critical).then(|| {
                (
                    p.facts.name.as_str(),
                    p.facts.friendly_name(),
                    input.activity.usage(&p.facts.key).core_pct,
                    c.in_foreground_tree,
                )
            })
        })
        .collect();
    by_cpu.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut by_mem: Vec<(String, u64)> = snap
        .processes
        .iter()
        .map(|p| (p.facts.friendly_name(), p.metrics.private_bytes))
        .collect();
    by_mem.sort_by_key(|(_, b)| std::cmp::Reverse(*b));

    let hot = [input.thermals.cpu_temp, input.thermals.gpu_temp]
        .into_iter()
        .flatten()
        .any(|t| t >= cfg.hot_temp);
    let top_background = by_cpu.iter().find(|(_, _, _, fg)| !fg);
    let top_foreground = by_cpu.iter().find(|(_, _, _, fg)| *fg);
    let cause = if hot || is_throttling(cfg, &snap.system, cpu_pct) {
        StutterCause::Thermal
    } else if cpu_pct >= 80.0
        && let Some((_, name, pct, _)) = top_background
        && *pct >= 30.0
    {
        StutterCause::BackgroundCpu {
            name: name.clone(),
            core_pct: *pct,
        }
    } else if snap.system.commit_pct() >= 90.0 || snap.system.mem_used_pct() >= 92.0 {
        StutterCause::MemoryPressure
    } else if !input.suspects.is_empty() {
        StutterCause::Leaks {
            count: input.suspects.len(),
        }
    } else if cpu_pct >= 80.0
        && let Some((_, name, _, _)) = top_foreground
    {
        StutterCause::ForegroundHeavy { name: name.clone() }
    } else {
        StutterCause::Unclear
    };

    let cpus = snap.system.logical_cpus.max(1) as f32;
    StutterReport {
        cause,
        cpu_pct,
        mem_used_pct: snap.system.mem_used_pct(),
        top_cpu: by_cpu
            .into_iter()
            .map(|(_, n, p, _)| (n, p / cpus))
            .filter(|(_, pct)| *pct >= MIN_LISTED_CPU_PCT)
            .take(5)
            .collect(),
        top_memory: by_mem.into_iter().take(5).collect(),
    }
}
