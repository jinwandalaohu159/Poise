//! 调度需要跨时间记住的事，各在一个文件里：前台保持、正在玩的游戏、发声保持、程序负载、资源需求、
//! 后台忙碌、临时全速。

mod audio;
mod boosts;
mod busy;
mod demand;
mod focus;
mod foreground;
mod games;
mod load;

pub use audio::AudioTracker;
pub use boosts::TempBoosts;
pub use busy::{BusyRule, BusyTracker, Demand};
pub use demand::{DemandRule, DemandSignals, DemandTracker, Pressure};
pub use focus::{FocusRule, FocusTracker, Footprint};
pub use foreground::{ForegroundHold, ForegroundTracker};
pub use games::GameTracker;
pub use load::{LoadSmoothing, TreeLoad};

/// 测试用：进程 10 和 20 各是一个程序，`fg_root` 在前台。
#[cfg(test)]
pub(crate) fn test_classes(
    fg_root: Option<crate::model::Pid>,
) -> std::collections::HashMap<crate::model::Pid, crate::classify::Classification> {
    use crate::classify::{Classification, Role, ScheduleScope};
    use crate::prefs::AppPreference;

    let mut m = std::collections::HashMap::new();
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
