//! 调度决定的日志：需求的变化、哪个程序因为什么被调整或恢复、重点程序占用的核心变化、
//! 让到小核的程序汇总。只负责写成给人看的文字，决定本身见 `poise_core::schedule`。

use poise_core::action::CoreSet;
use poise_core::model::Snapshot;
use poise_core::schedule::{Cause, Decision, Pressure, ProgramState, Verdict};

/// 记录调度决定的变化：需求的变化，以及哪个程序因为什么被调整、恢复。
pub(super) fn log(decisions: &[Decision], snapshot: &Snapshot) {
    for d in decisions {
        match d {
            Decision::Pressure(p) => log::info!(
                "处理器需求：{}",
                match p {
                    Pressure::None => "无（后台不必让路）",
                    Pressure::Normal => "普通（正在用的程序在忙，后台只降优先级）",
                    Pressure::High => "高（有重点程序或发热，后台完整让路）",
                }
            ),
            Decision::Aside(0) => log::info!("没有重点程序了：让到小核的程序移回所有核心"),
            Decision::Aside(n) => {
                log::info!("{n} 个没在用的程序让到小核（按缓存分组），大核留给重点程序");
            }
            Decision::Program { root, from, state } => {
                let name = snapshot
                    .find(root.pid)
                    .map_or_else(|| root.pid.to_string(), |p| p.facts.friendly_name());
                log::info!(
                    "{name}（进程 {}，{}）：{}",
                    root.pid,
                    load_text(state),
                    what_text(*from, state)
                );
            }
        }
    }
}

fn load_text(state: &ProgramState) -> String {
    if state.disk_mb_s == 0 {
        format!("负载 {}%", state.load_pct)
    } else {
        format!("负载 {}%，磁盘 {} MB/s", state.load_pct, state.disk_mb_s)
    }
}

fn what_text(from: Verdict, state: &ProgramState) -> String {
    let what = match state.verdict {
        Verdict::Playing => format!("按游戏调度，{}", cores_text(state.cores)),
        Verdict::Focused => format!("持续高负载，{}", cores_text(state.cores)),
        Verdict::Yielded if state.cause == Some(Cause::MarkedLow) => "让路".into(),
        Verdict::Yielded => "后台忙，让路（移到小核、节能、降优先级）".into(),
        Verdict::Lowered => "后台忙，降低优先级（空闲的核心照样用）".into(),
        Verdict::Backseat => "后台忙，移到小核（不降优先级）".into(),
        Verdict::Aside => "移到小核".into(),
        Verdict::Normal if from == Verdict::Playing => "不再按游戏调度".into(),
        Verdict::Normal if from == Verdict::Focused => "不再优先用大核".into(),
        Verdict::Normal => "恢复".into(),
    };
    match state.cause {
        Some(cause) => format!("{what}（{}）", cause_text(cause)),
        None => what,
    }
}

/// 重点程序用哪些核心（核心域见 `CoreSet`：主域是大核，其余是按缓存分的组）。
fn cores_text(cores: CoreSet) -> String {
    match cores {
        CoreSet::All => "用所有核心".into(),
        CoreSet::Performance | CoreSet::Domains(1) => "用大核".into(),
        CoreSet::Domains(m) if m & 1 != 0 => {
            format!("用大核和 {} 组共用缓存的核心", (m >> 1).count_ones())
        }
        _ => "用小核".into(),
    }
}

fn cause_text(cause: Cause) -> &'static str {
    match cause {
        Cause::Foreground => "在前台",
        Cause::RecentlyUsed => "刚用过",
        Cause::SelfElevated => "程序自己设为高优先级",
        Cause::MarkedLow => "设为低优先级",
        Cause::Audio => "在发声",
        Cause::Boosted => "临时全速",
        Cause::Marked => "设为始终优先或游戏",
        Cause::NoDemand => "没人需要处理器",
        Cause::LessDemand => "需求降低",
        Cause::Exited => "已退出",
    }
}
