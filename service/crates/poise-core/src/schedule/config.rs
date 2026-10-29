//! 调度参数与处理器的大小核构成。

use super::trackers::{BusyRule, DemandRule, ForegroundHold};
use crate::action::CoreSet;
use crate::model::Millis;

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// 失去前台后多久内仍算"正在用"、最多保持几个程序。
    pub foreground_hold: ForegroundHold,
    /// 程序负载的平滑时间常数：短于它的尖峰影响很小。
    pub load_smoothing_ms: Millis,
    /// 什么情况下算有人需要处理器（没人需要时后台不让路）。
    pub demand: DemandRule,
    /// 后台程序怎样算"在忙"、要让路。
    pub busy: BusyRule,
    /// 玩游戏时，发声的程序怎样算"在忙"、要让到小核。门槛高得多：只有真的很忙才值得挪，
    /// 放着歌的浏览器这类轻负载不动（切来切去也不会跟着来回调整）。
    pub busy_with_audio: BusyRule,
    /// 最后一个游戏退出后，游戏模式再保持多久（一局结束到下一局开始之间不切回日常模式）。
    pub game_session_hold_ms: Millis,
    /// 停声后多久内仍算"在发声"。
    pub audio_hold_ms: Millis,
    /// 自动识别的游戏（全屏且在渲染）要持续多久才确认为"正在玩"。
    pub game_confirm_ms: Millis,
    /// 处理器的大小核构成。
    pub cores: CoreLayout,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            foreground_hold: ForegroundHold {
                ms: 5 * 60_000,
                max_apps: 3,
            },
            load_smoothing_ms: 30_000,
            demand: DemandRule {
                foreground_core_pct: 25.0,
                system_pct: 50.0,
                hold_ms: 30_000,
            },
            busy: BusyRule {
                core_pct: 10.0,
                core_pct_high: 3.0,
                release_after_ms: 60_000,
            },
            busy_with_audio: BusyRule {
                core_pct: 25.0,
                core_pct_high: 25.0,
                release_after_ms: 60_000,
            },
            game_session_hold_ms: 3 * 60_000,
            audio_hold_ms: 30_000,
            game_confirm_ms: 6_000,
            cores: CoreLayout {
                performance_threads: 12,
                efficiency_threads: 8,
            },
        }
    }
}

/// 处理器的大小核构成（线程数）。不是大小核处理器时全部算大核，小核为 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreLayout {
    pub performance_threads: u32,
    pub efficiency_threads: u32,
}

impl CoreLayout {
    /// 小核至少要有这么多线程才把后台让到小核：太少时一堆后台程序挤在一两个核心上互相拖慢，
    /// 只用优先级和节能让路就够了。
    const MIN_EFFICIENCY_THREADS: u32 = 4;

    /// 所有核心同一等级（不是大小核处理器）。
    pub fn uniform(threads: u32) -> Self {
        Self {
            performance_threads: threads,
            efficiency_threads: 0,
        }
    }

    pub fn hybrid(&self) -> bool {
        self.performance_threads > 0 && self.efficiency_threads > 0
    }

    /// 能不能把核心范围限制到 `set`。
    pub(super) fn allows(&self, set: CoreSet) -> bool {
        match set {
            CoreSet::All => true,
            CoreSet::Performance => self.hybrid(),
            CoreSet::Efficiency => {
                self.hybrid() && self.efficiency_threads >= Self::MIN_EFFICIENCY_THREADS
            }
        }
    }
}
