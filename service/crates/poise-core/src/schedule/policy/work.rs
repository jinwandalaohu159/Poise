//! 用户的程序在干活时（见 `trackers::work`）应有的状态：能用多少处理器由调度判断（见 `working::rooms`），
//! 这里按它换成具体的状态。

use super::ProcessView;
use super::in_use;
use crate::action::CoreSet;
use crate::schedule::desired::Desired;

/// 在干活的程序能用多少处理器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkRoom {
    /// 用户正在用它，或者没人和它抢：全用。
    #[default]
    Free,
    /// 正在用的别的程序在忙：核心照用，优先级让一步。
    Contended,
    /// 别的程序是重点程序（前台在干重活、在玩游戏）：用它没留下的核心（见 `working::WorkRooms`）——
    /// 大小核处理器上，系统优先使用的几个大核（按物理核心）留给它，其余大核和全部小核给这份工作
    /// （`CoreSet::Rest`）；副域一样快的处理器（多 CCD）上，整个主域留给它，这份工作用别的 CCD，
    /// 不和它抢同一块缓存。
    Shared(CoreSet),
    /// 重点程序要整个大核（它用得多，或玩游戏时处理器发热、被功耗或温度压着）：移到小核。
    Aside,
}

/// 在干活的程序应有的状态（见 [`Desired::WORK`]）。用电池时不替系统做主：正在用的不调整，其余只降优先级。
/// 在发声、自己把优先级调到"高"的程序（通话、录像、串流）让核心不让优先级：降了声音和画面会断续。
pub(super) fn work_state(v: &ProcessView) -> Desired {
    let mut d = if v.on_battery {
        if in_use(v) {
            Desired::NEUTRAL
        } else {
            Desired::LOWERED
        }
    } else {
        match v.work_room {
            WorkRoom::Free => Desired::WORK,
            WorkRoom::Contended => Desired::WORK_SHARED,
            WorkRoom::Shared(cores) => Desired {
                cores,
                ..Desired::WORK_SHARED
            },
            WorkRoom::Aside => Desired::WORK_ASIDE,
        }
    };
    if v.audible || v.self_elevated {
        d.priority = None;
    }
    d
}
