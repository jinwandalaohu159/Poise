//! 卡顿归因：刚才为什么卡。

use crate::fast_hash::HashMap;
use serde::{Deserialize, Serialize};

use crate::activity::ActivityTracker;
use crate::classify::{Classification, Role};
use crate::cleanup::LeakSuspect;
use crate::model::{Pid, Snapshot};

use super::{DiagnoseConfig, Thermals, is_throttling};

/// 占整机 CPU 低于这个百分比的程序不列出（列出来也说明不了问题）。
const MIN_LISTED_CPU_PCT: f32 = 0.5;

/// 卡顿原因的结论。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StutterCause {
    /// 过热或功耗墙导致降频。
    Thermal,
    /// 后台程序在抢 CPU。`share_pct` 是占整机的百分比；`path` 用于"设为低优先级"。
    BackgroundCpu {
        name: String,
        path: Option<String>,
        share_pct: f32,
    },
    /// 内存不够用了。
    MemoryPressure,
    /// 有程序越用越占资源。
    Leaks { count: usize },
    /// 前台程序本身就很重。
    ForegroundHeavy { name: String, share_pct: f32 },
    /// 没发现明显问题（可能是资源管理器插件等）。
    Unclear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StutterReport {
    pub cause: StutterCause,
    pub cpu_pct: f32,
    pub mem_used_pct: f32,
    pub cpu_temp: Option<f32>,
    /// 处理器当前频率是否明显低于基础频率（散热或功耗限制）。
    pub throttling: bool,
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

struct CpuUser {
    name: String,
    path: Option<String>,
    /// 占整机的百分比。
    share_pct: f32,
    foreground: bool,
}

pub fn analyze_stutter(cfg: &DiagnoseConfig, input: &StutterInput) -> StutterReport {
    let snap = input.snapshot;
    let cpu_pct = input.activity.system_cpu_pct();
    let cpus = snap.system.logical_cpus.max(1) as f32;
    let mut by_cpu: Vec<CpuUser> = snap
        .processes
        .iter()
        .filter_map(|p| {
            let c = input.classes.get(&p.facts.key.pid)?;
            (c.role != Role::Critical).then(|| CpuUser {
                name: p.facts.friendly_name(),
                path: p.facts.image_path.clone(),
                share_pct: input
                    .activity
                    .usage(&p.facts.key)
                    .share_pct(snap.system.logical_cpus),
                foreground: c.in_foreground_tree,
            })
        })
        .collect();
    by_cpu.sort_by(|a, b| b.share_pct.total_cmp(&a.share_pct));
    let mut by_mem: Vec<(String, u64)> = snap
        .processes
        .iter()
        .map(|p| (p.facts.friendly_name(), p.metrics.memory_bytes))
        .collect();
    by_mem.sort_by_key(|(_, b)| std::cmp::Reverse(*b));

    let hot = [input.thermals.cpu_temp, input.thermals.gpu_temp]
        .into_iter()
        .flatten()
        .any(|t| t >= cfg.hot_temp);
    let throttling = is_throttling(cfg, &snap.system, cpu_pct);
    let top_background = by_cpu.iter().find(|u| !u.foreground);
    let top_foreground = by_cpu.iter().find(|u| u.foreground);
    let cause = if hot || throttling {
        StutterCause::Thermal
    } else if cpu_pct >= 80.0
        && let Some(u) = top_background
        && u.share_pct * cpus >= 30.0
    {
        StutterCause::BackgroundCpu {
            name: u.name.clone(),
            path: u.path.clone(),
            share_pct: u.share_pct,
        }
    } else if snap.system.commit_pct() >= 90.0 || snap.system.mem_used_pct() >= 92.0 {
        StutterCause::MemoryPressure
    } else if !input.suspects.is_empty() {
        StutterCause::Leaks {
            count: input.suspects.len(),
        }
    } else if cpu_pct >= 80.0
        && let Some(u) = top_foreground
    {
        StutterCause::ForegroundHeavy {
            name: u.name.clone(),
            share_pct: u.share_pct,
        }
    } else {
        StutterCause::Unclear
    };

    StutterReport {
        cause,
        cpu_pct,
        mem_used_pct: snap.system.mem_used_pct(),
        cpu_temp: input.thermals.cpu_temp,
        throttling,
        top_cpu: by_cpu
            .into_iter()
            .map(|u| (u.name, u.share_pct))
            .filter(|(_, pct)| *pct >= MIN_LISTED_CPU_PCT)
            .take(5)
            .collect(),
        top_memory: by_mem.into_iter().take(5).collect(),
    }
}
