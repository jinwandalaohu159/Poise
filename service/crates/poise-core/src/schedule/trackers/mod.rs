//! 调度需要跨时间记住的事，各在一个文件里：前台保持、正在玩的游戏、发声保持、程序负载、资源需求、
//! 后台忙碌、重点程序与它占用的核心、临时全速。

mod audio;
mod boosts;
mod busy;
mod demand;
mod focus;
mod footprint;
mod foreground;
mod games;
mod load;

pub use audio::AudioTracker;
pub use boosts::TempBoosts;
pub use busy::{BusyRule, BusyTracker, Demand};
pub use demand::{DemandReason, DemandRule, DemandSignals, DemandTracker, Pressure};
pub use focus::{FocusEnd, FocusRule, FocusState, FocusTracker};
pub use footprint::{FocusLoad, Footprint};
pub use foreground::{ForegroundHold, ForegroundTracker};
pub use games::GameTracker;
pub use load::{LoadSmoothing, TreeLoad};

/// 进程的创建时间（快照里找不到时为 None）。按进程号记的保持状态用它核对：进程号被别的程序用上时不继承。
pub(crate) fn created_of(snap: &crate::model::Snapshot, pid: crate::model::Pid) -> Option<u64> {
    snap.find(pid).map(|p| p.facts.key.created)
}

/// 测试用：进程 10 和 20 各是一个程序，`fg_root` 在前台。
#[cfg(test)]
pub(crate) fn test_classes(
    fg_root: Option<crate::model::Pid>,
) -> crate::fast_hash::HashMap<crate::model::Pid, crate::classify::Classification> {
    use crate::classify::{Classification, Role, ScheduleScope};
    use crate::prefs::AppPreference;

    let mut m = crate::fast_hash::HashMap::default();
    for root in [10, 20] {
        m.insert(
            root,
            Classification {
                role: Role::UserApp,
                root_pid: root,
                in_foreground_tree: fg_root == Some(root),
                has_visible_window: true,
                tree_has_window: false,
                plays_audio: false,
                emits_audio: false,
                recently_active: false,
                preference: AppPreference::Auto,
                is_game: false,
                anti_cheat_present: false,
                restartable_service: None,
                schedule: ScheduleScope::App,
            },
        );
    }
    m
}
