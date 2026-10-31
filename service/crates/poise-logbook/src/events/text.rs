//! 事件在文本日志 poise.log 里的写法（中文，给排查问题的人看）。界面按自己的语言另外组织文字。

use log::Level;
use poise_core::power_settings::PowerSettingKind;
use poise_core::prefs::AppPreference;
use poise_core::schedule::{Cause, DemandReason, Pressure, Verdict};
use poise_protocol::{CleanupTrigger, CoreSpan, Event, ProgramRef};

/// 一条事件的级别和文字。
pub(super) fn describe(event: &Event) -> (Level, String) {
    let info = |s: String| (Level::Info, s);
    match event {
        Event::Demand { level, reason } => info(demand_text(*level, *reason)),
        Event::Program {
            program,
            from,
            to,
            cores,
            load_pct,
            disk_mb_s,
            cause,
        } => info(format!(
            "{}（{}）：{}",
            program_text(program),
            load_text(*load_pct, *disk_mb_s),
            what_text(*from, *to, *cores, *cause)
        )),
        Event::MovedAside { count: 0 } => {
            info("没有重点程序了：让到小核的程序移回所有核心".into())
        }
        Event::MovedAside { count } => info(format!(
            "{count} 个没在用的程序让到小核（按缓存分组），大核留给重点程序"
        )),
        Event::AccessDenied { program } => info(format!(
            "{}：系统拒绝访问，不再尝试调整",
            program_text(program)
        )),
        Event::Recovered { left, restored } => info(format!(
            "上次异常退出，留下 {left} 个调整过的进程，已把还在运行的 {restored} 个恢复原样"
        )),
        Event::ModeChanged { mode, on_battery } => info(format!(
            "模式切换为 {mode:?}{}",
            if *on_battery { "（用电池）" } else { "" }
        )),
        Event::ModeFailed { mode } => (Level::Warn, format!("切换到 {mode:?} 模式失败")),
        Event::Throttled { limit_pct } => info(match limit_pct {
            Some(p) => format!("处理器正被限速（性能上限 {p:.0}%），后台更早让路"),
            None => "处理器正被限速，后台更早让路".into(),
        }),
        Event::Unthrottled => info("处理器不再被限速".into()),
        Event::PlanSwitched => info("电源计划换了，当前模式已重新写入新计划".into()),
        Event::SettingRestored {
            setting,
            class,
            ac,
            found,
            expected,
        } => info(format!(
            "{}（{}）被别处改成了 {}（Poise 设的是 {}），已改回",
            setting_text(*setting, *class),
            if *ac { "插电" } else { "电池" },
            value_text(*found),
            value_text(*expected)
        )),
        Event::OptimizationReapplied { id } => {
            info(format!("优化项 {id} 已不是优化状态，已重新应用"))
        }
        Event::EfficiencySlider => info(
            "Windows 电源模式处于\"最佳能效\"：它会盖住电源计划里的能效偏好等设置，Poise 的模式只部分生效"
                .into(),
        ),
        Event::OptimizationsApplied { ids } => info(format!("应用优化项：{}", ids.join("、"))),
        Event::OptimizationsReverted { ids } => info(format!("还原优化项：{}", ids.join("、"))),
        Event::Cleaned {
            trigger,
            freed_bytes,
            handled,
            failed,
        } => info(format!(
            "{}：处理 {handled} 项，释放 {} MB{}",
            match trigger {
                CleanupTrigger::Manual => "清理",
                CleanupTrigger::Auto => "自动清理",
                CleanupTrigger::Deferred => "补做延后的清理",
            },
            freed_bytes / (1024 * 1024),
            if *failed > 0 {
                format!("，{failed} 项未完成")
            } else {
                String::new()
            }
        )),
        Event::Started {
            version,
            observe_only,
        } => info(format!(
            "Poise {version} 启动{}",
            if *observe_only {
                "（观察模式，不执行）"
            } else {
                ""
            }
        )),
        Event::Resumed => info("托盘代理已连接，开始调度".into()),
        Event::Paused => info("托盘代理已退出：进程调整、散热与电源计划已全部还原".into()),
        Event::PreferenceSet {
            name, preference, ..
        } => info(format!(
            "{name} 设为{}",
            match preference {
                AppPreference::Auto => "自动",
                AppPreference::AlwaysFast => "始终优先",
                AppPreference::Background => "低优先级",
                AppPreference::Game => "游戏",
            }
        )),
        Event::SmartSchedule { enabled } => info(format!(
            "智能调度{}",
            if *enabled { "已打开" } else { "已关闭，进程调整已撤销" }
        )),
    }
}

fn demand_text(level: Pressure, reason: Option<DemandReason>) -> String {
    let what = match level {
        Pressure::None => "无（后台不必让路）",
        Pressure::Normal if reason == Some(DemandReason::Battery) => "普通（用电池，后台让路省电）",
        Pressure::Normal => "普通（正在用的程序在忙，后台只降优先级）",
        Pressure::High if reason == Some(DemandReason::Focus) => "高（有重点程序，后台完整让路）",
        Pressure::High if reason == Some(DemandReason::Heat) => {
            "高（处理器没有余量，后台完整让路）"
        }
        Pressure::High => "高（后台完整让路）",
    };
    format!("处理器需求：{what}")
}

fn program_text(p: &ProgramRef) -> String {
    format!("{}（进程 {}）", p.name, p.pid)
}

fn load_text(load_pct: u32, disk_mb_s: u32) -> String {
    if disk_mb_s == 0 {
        format!("负载 {load_pct}%")
    } else {
        format!("负载 {load_pct}%，磁盘 {disk_mb_s} MB/s")
    }
}

fn what_text(from: Verdict, to: Verdict, cores: CoreSpan, cause: Option<Cause>) -> String {
    let what = match to {
        Verdict::Playing => format!("按游戏调度，{}", cores_text(cores)),
        Verdict::Focused => format!("成为重点程序，{}", cores_text(cores)),
        Verdict::Yielded if cause == Some(Cause::MarkedLow) => "让路".into(),
        Verdict::Yielded => "后台忙，让路（移到小核、节能、降优先级）".into(),
        Verdict::Lowered => "后台忙，降低优先级（空闲的核心照样用）".into(),
        Verdict::Backseat => "后台忙，移到小核（不降优先级）".into(),
        Verdict::Aside => "移到小核".into(),
        Verdict::Normal if from == Verdict::Playing => "不再按游戏调度".into(),
        Verdict::Normal if from == Verdict::Focused => "不再是重点程序".into(),
        Verdict::Normal => "恢复".into(),
    };
    match cause {
        Some(cause) => format!("{what}（{}）", cause_text(cause)),
        None => what,
    }
}

/// 用哪些核心（主域是大核，其余是按缓存分的组）。
fn cores_text(cores: CoreSpan) -> String {
    match cores {
        CoreSpan { all: true, .. } => "用所有核心".into(),
        CoreSpan {
            main: true,
            others: 0,
            ..
        } => "用大核".into(),
        CoreSpan {
            main: true, others, ..
        } => format!("用大核和 {others} 组共用缓存的核心"),
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
        Cause::LooksLikeGame => "像游戏，等待确认",
        Cause::LoadDropped => "负载降下来了",
        Cause::LeftForeground => "离开前台",
    }
}

fn setting_text(setting: Option<PowerSettingKind>, class: Option<u8>) -> String {
    let label = match setting {
        Some(PowerSettingKind::MinState) => "最小处理器状态",
        Some(PowerSettingKind::MaxState) => "最大处理器状态",
        Some(PowerSettingKind::Epp) => "能效偏好",
        Some(PowerSettingKind::CoreParkingMin) => "核心停放最少核心",
        Some(PowerSettingKind::BoostMode) => "睿频模式",
        None => "电源设置",
    };
    match class {
        Some(c) => format!("{label}（等级 {c}）"),
        None => label.into(),
    }
}

fn value_text(v: Option<u32>) -> String {
    v.map_or_else(|| "（不存在）".into(), |n| n.to_string())
}
