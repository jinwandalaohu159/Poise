//! 调度参数。处理器的核心域见 `layout`。

use super::CoreLayout;
use super::trackers::{BusyRule, DemandRule, FocusRule, ForegroundHold, LoadSmoothing};
use crate::model::Millis;

const MB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// 失去前台后多久内仍算"正在用"、最多保持几个程序。
    pub foreground_hold: ForegroundHold,
    /// 程序负载的平滑时间常数（见 `trackers::LoadSmoothing`）。
    pub load_smoothing: LoadSmoothing,
    /// 什么情况下算有人需要处理器（没人需要时后台不让路）。
    pub demand: DemandRule,
    /// 后台程序怎样算"在忙"、要让路。
    pub busy: BusyRule,
    /// 有重点程序时，发声的程序怎样算"在忙"、要让到小核。门槛高得多：只有真的很忙才值得挪，
    /// 放着歌的浏览器这类轻负载不动（切来切去也不会跟着来回调整）。
    pub busy_with_audio: BusyRule,
    /// 什么样的程序算重点程序（大核留给它，其余没在用的程序让到小核，见 `trackers::focus`）。
    pub focus: FocusRule,
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
            load_smoothing: LoadSmoothing {
                // 判断后台在不在忙：短于半分钟的尖峰影响很小。
                slow_ms: 30_000,
                // 判断正在用的程序是不是在干重活：单线程跑满 10 秒内就能认出来。
                fast_ms: 5_000,
            },
            demand: DemandRule {
                in_use_core_pct: 25.0,
                system_pct: 50.0,
                in_use_active_pct: 5.0,
                hold_ms: 30_000,
            },
            busy: BusyRule {
                core_pct: 10.0,
                core_pct_high: 3.0,
                disk_bytes_per_s: 8.0 * MB,
                release_after_ms: 60_000,
                reyield_after_ms: 20_000,
            },
            busy_with_audio: BusyRule {
                core_pct: 25.0,
                core_pct_high: 25.0,
                // 发声的程序（播放、通话）本来就会读写，只看处理器。
                disk_bytes_per_s: f64::INFINITY,
                release_after_ms: 60_000,
                reyield_after_ms: 20_000,
            },
            focus: FocusRule {
                // 持续占用接近一个核心：编译、渲染、导出、游戏这类工作；浏览网页、看视频远低于此。
                enter_core_pct: 80.0,
                leave_core_pct: 40.0,
                leave_after_ms: 10_000,
                // 平滑后的负载持续 8 秒达到门槛：单线程重活大约 15 秒认出来，网页加载、打开聊天窗口不算。
                enter_after_ms: 8_000,
                away_grace_ms: 15_000,
                hold_ms: 20_000,
                work_settle_ms: 60_000,
            },
            game_session_hold_ms: 3 * 60_000,
            audio_hold_ms: 30_000,
            game_confirm_ms: 6_000,
            // 开发机（i7-12700H）：6 个大核 12 线程，小核两组各 4 个。实际运行时按检测到的拓扑。
            cores: CoreLayout::with_cores(&[(12, 6), (4, 4), (4, 4)]),
        }
    }
}
