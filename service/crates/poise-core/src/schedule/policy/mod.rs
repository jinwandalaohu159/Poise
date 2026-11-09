//! 调度策略：根据一个进程的情况，决定它"应该"处于什么状态。纯函数，不记状态。
//!
//! 原则是**让路为主，基本不提速**：大幅提速会和声音、输入、系统组件抢资源，反而卡顿。Poise 主要把在后台
//! 持续占用处理器的程序挪开，让正在用的程序自然拿到资源。唯一的提速是用户标记为"游戏"的程序：
//! 提高一级（正常 → 高于正常），不会更高。
//!
//! 但"在后台"不等于"是后台程序"。先按同一条规则分清每个程序归谁（见 `trackers::work`）：用户用过的、
//! 由它们启动的、正在为它们干活的是用户的，其余是后台自己的。用户的程序在忙就是在干活：不让路，反而
//! 明确不让系统自动节能（Windows 会把没有前台窗口的程序放到小核），只在有竞争时分处理器——正在用的
//! 别的程序在忙时优先级让一步，有重点程序时大核按物理核心切开分享，重点程序要整个大核时才移到小核。
//! 玩游戏时例外：游戏要的不是核心而是频率，频率取决于所有核心共用的功耗，后台的工作移到小核并节能。
//!
//! 另外弥补 Windows 的短板：
//! - Windows 会自动给后台、最小化的程序降频、放宽计时器精度，游戏切出去、后台音乐和语音会因此卡顿。
//!   对这些程序明确关闭系统的自动节能。
//! - Windows 有时把重点程序的线程放到慢的核心上（只靠一两个主线程的游戏，主线程落到小核帧数就掉），
//!   又让别的程序的零碎工作打断重点程序。重点程序（正在玩的游戏，以及正被直接使用、持续需要处理器的
//!   任何程序，见 `trackers::focus`）按需要的线程占用主域（大核），不够再逐个加上副域；其余没在用的
//!   程序让到副域，同一个程序固定在同一个域里（见 `placement`）。核心域见 `CoreSet`：大小核处理器上
//!   主域是大核、副域是按缓存分组的小核；多 CCD 的处理器上是各个 CCD；只有一个域的处理器不限制核心。
//!
//! 下表的"大核""小核"指主域、副域。
//!
//! | 情况 | 状态 |
//! |---|---|
//! | 用户标记为"游戏" | 大核 · 优先级高一级 · 不受系统节能限速 |
//! | 正在玩的游戏（自动识别） | 大核 · 不受系统节能限速 |
//! | 正被直接使用、持续需要处理器的程序（前台的编译、渲染等） | 大核，其余照常 |
//! | 上面几种重点程序，需要的线程超过大核能提供的 | 逐个加上副域，直到不限核心 |
//! | 始终优先、临时全速 | 不受系统节能限速 |
//! | 正在发声的程序（整个进程树，停声后保持一会儿）里输出声音的进程 | 不受系统节能限速 |
//! | 正在发声的程序里的其他进程 | 不调整（不让路，也不替程序做主：浏览器会自己给后台标签页节能） |
//! | 正在发声、有重点程序时又在后台忙 | 小核 · 不受系统节能限速（声音照样流畅，大核留给重点程序） |
//! | 用户的程序（用过的、由它们启动的、正在为它们干活的）在忙，正在用它或没人和它抢 | 不限核心 · 不受系统节能限速（否则 Windows 把它当后台放到小核） |
//! | 同上，正在用的别的程序在忙 | 低于正常 · 不限核心 · 不受系统节能限速 |
//! | 同上，有别的重点程序 | 同上，但重点程序实际要几个大核就留几个（优选核心在前，按物理核心），用其余核心；副域一样快的处理器（多 CCD）上整个主域留给它，用别的 CCD，不抢缓存 |
//! | 同上，重点程序要整个大核，或有重点程序时处理器发热、被功耗或温度压着（缓过来两分钟后才回去） | 小核 · 低于正常 |
//! | 同上，重点程序里有正在玩的游戏 | 小核 · 低于正常 · 节能（功耗留给游戏的主线程） |
//! | 同上，用电池时 | 低于正常（核心不限；正在用的不调整） |
//! | 上面几种里在发声、自己把优先级调到"高"的程序（通话、录像、串流） | 只让核心，不降优先级、不节能 |
//! | 前台、刚用过的程序 | 不调整（Windows 本来就照顾前台）；玩游戏时"刚用过"不再豁免，只有前台算 |
//! | 自己把优先级调到"高"的程序（录屏、串流、音频工作站开工时会这样做）在后台忙 | 不调整；有重点程序时只让到小核，同发声的程序 |
//! | 低优先级（用户指定） | 小核 · 低于正常 · 在后台节能 |
//! | 自动，在后台持续占用处理器，有重点程序、发热或用电池时 | 小核 · 低于正常 · 节能（保持到它回到前台、发声或退出） |
//! | 同上，但负载轻（达不到需求一般时的门槛：手柄映射、录像、副屏视频） | 小核 · 低于正常（不节能：节能会让输入变迟、画面掉帧） |
//! | 自动，在后台持续占用处理器，只是正在用的程序在忙 | 低于正常（核心不限，节能交给 Windows） |
//! | 自动，让路之后没人需要处理器了、它还在忙 | 低于正常（同上；闲下来才恢复，不来回调整） |
//! | 自动，在后台闲着，有重点程序时 | 小核（不降优先级、不节能；大核留给重点程序） |
//! | 自动，在后台闲着 | 不调整（闲着的程序不和谁抢资源，不值得动它） |
//!
//! 系统进程不在调度范围内。

mod work;

use super::decisions::Verdict;
use super::desired::Desired;
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::prefs::AppPreference;
pub use work::WorkRoom;
use work::work_state;

/// 单个进程的情况。
#[derive(Debug, Clone, Copy)]
pub struct ProcessView<'a> {
    pub class: &'a Classification,
    /// 属于正在玩的游戏（游戏本身及其进程树，包括切到后台的游戏）。
    pub active_game: bool,
    /// 有游戏正在运行：此时只有前台算正在用，刚用过的程序（例如启动游戏的客户端）不再豁免。
    pub gaming: bool,
    /// 它是重点程序时，按它需要的线程算出的核心范围（见 `trackers::Footprint`）。
    pub focus: Option<CoreSet>,
    /// 有重点程序（或刚结束不久）：没在用的程序让到小核。
    pub focusing: bool,
    /// 刚失去前台不久（保持期内）。
    pub recently_foreground: bool,
    /// 用户临时全速。
    pub boosted: bool,
    /// 进程树正在发声，或刚停声不久（由调度器跨周期判断，切歌、暂停的间隙不来回切换）。
    pub audible: bool,
    /// 这个进程本身在输出声音，或刚停下不久。
    pub emitting: bool,
    /// 程序的主进程自己把优先级调到了"高"或更高：它在声明自己的工作对时间敏感。
    pub self_elevated: bool,
    /// 在后台持续占用处理器（由调度器跨周期判断）。
    pub busy: bool,
    /// 在忙，但负载轻（见 `BusyTracker::heavy`）：完整让路时不节能。
    pub light: bool,
    /// 正在发声，有重点程序时又在后台忙（由调度器跨周期判断）：只让到小核。
    pub busy_with_audio: bool,
    /// 刚停止使用不久：先不让到小核，来回切换窗口、通知弹窗抢一下前台不会让它来回挪动。
    pub settling: bool,
    /// 在忙的程序完整让路（需求高、用电池时），否则只降优先级。
    pub yield_fully: bool,
    /// 用户的程序在干活（由调度器跨周期判断，见 `trackers::work`）。
    pub working: bool,
    /// 在干活时能用多少处理器（由调度器判断，见 `working`）。
    pub work_room: WorkRoom,
    pub on_battery: bool,
}

/// 正在被用户使用、绝不让路的程序：用户直接在用的（见 `Classification::in_use_by_user`，安全守卫
/// 按它再把关一次），加上跨周期才看得出来的——刚用过、正在玩、临时全速、在发声。
pub fn in_use(v: &ProcessView) -> bool {
    v.class.in_use_by_user()
        || (v.recently_foreground && !v.gaming)
        || v.active_game
        || v.boosted
        || v.audible
}

/// 用户要求保持快的程序：不让系统自动节能限速。
fn kept_fast(v: &ProcessView) -> bool {
    v.boosted
        || matches!(
            v.class.preference,
            AppPreference::AlwaysFast | AppPreference::Game
        )
}

/// 进程应有的状态，以及按什么调度（供日志）。
pub fn decide(v: &ProcessView) -> (Desired, Verdict) {
    let (mut desired, mut verdict) = decide_cores_freely(v);
    // 重点程序：在大核上（按需要逐级放开）。用户设为"低优先级"的程序除外，照用户的留在小核。
    if let Some(cores) = v.focus
        && v.class.is_schedulable()
        && matches!(desired.cores, CoreSet::All | CoreSet::Performance)
    {
        desired.cores = cores;
        if matches!(verdict, Verdict::Normal | Verdict::Working) {
            verdict = Verdict::Focused;
        }
    }
    (desired, verdict)
}

fn decide_cores_freely(v: &ProcessView) -> (Desired, Verdict) {
    use Verdict::*;
    let c = v.class;
    if c.is_maintenance() {
        return if v.focusing && !in_use(v) {
            (Desired::MAINTENANCE_ASIDE, Aside)
        } else {
            (Desired::NEUTRAL, Normal)
        };
    }
    if !c.is_schedulable() {
        return (Desired::NEUTRAL, Normal);
    }
    if c.preference == AppPreference::Game {
        return (Desired::GAME, Playing);
    }
    if v.active_game {
        return (Desired::PLAYING, Playing);
    }
    if v.working {
        // 正在用的程序在干活不必报告（用户看得见它），切走以后才是"在后台干活"。状态照样不节能：
        // 来回切换窗口时不跟着来回调整。
        let verdict = if in_use(v) { Normal } else { Working };
        return (work_state(v), verdict);
    }
    if v.busy_with_audio {
        return (Desired::BACKSEAT, Backseat);
    }
    if kept_fast(v) {
        return (Desired::PROTECTED, Normal);
    }
    // 发声的程序连用户指定的"低优先级"也让步：输出声音的进程不受限速，其余进程不动。
    if v.audible {
        let desired = if v.emitting {
            Desired::PROTECTED
        } else {
            Desired::NEUTRAL
        };
        return (desired, Normal);
    }
    if c.preference == AppPreference::Background {
        return if c.in_foreground_tree {
            (Desired::LOW_IN_FOREGROUND, Yielded)
        } else {
            (Desired::YIELDED, Yielded)
        };
    }
    if v.busy && v.self_elevated && !in_use(v) {
        // 降优先级、节能限速都会让录像、串流掉帧；有重点程序时让到小核就够了，大核留给重点程序。
        return if v.focusing {
            (Desired::BACKSEAT, Backseat)
        } else {
            (Desired::NEUTRAL, Normal)
        };
    }
    if v.busy && !in_use(v) {
        if v.yield_fully && v.light {
            (Desired::NUDGED, Yielded)
        } else if v.yield_fully {
            (Desired::YIELDED, Yielded)
        } else {
            (Desired::LOWERED, Lowered)
        }
    } else if v.focusing && v.focus.is_none() && !v.settling && !in_use(v) {
        (Desired::ASIDE, Aside)
    } else {
        (Desired::NEUTRAL, Normal)
    }
}

#[cfg(test)]
mod tests;
