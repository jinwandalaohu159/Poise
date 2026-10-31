//! 采样节奏：多久采一轮。采集进程表是每轮最贵的一步（系统要列出所有线程，几千个线程要 3～5 毫秒，
//! 一轮其余的事合起来不到 1 毫秒），所以只在需要时采得勤：
//!
//! - 刚有变化（前台切换、全屏切换、调整了程序、决定变了）后的一会儿、界面显示着：每 [`TICK`]；
//! - 平静时，包括游戏玩得正稳、重点程序稳定地干活、全屏看视频：每 [`CALM_TICK`]——需要判断的都已判断完，
//!   负载按半分钟平滑，采得勤也不会更早做出决定；
//! - 用户离开、没在玩游戏、界面也没显示：每 [`IDLE_TICK`]；
//! - Poise 暂停着（托盘代理不在）、界面也没显示：每 [`PAUSED_TICK`]——不调度，只看着增长趋势；托盘代理一上报
//!   就立即恢复（见 `requests`，那时重新采一次），之后最多一个间隔回到平常的节奏；
//! - 熄屏、待机（见 `standby`）：每 [`STANDBY_TICK`]——没人在用桌面；亮屏时立即采一轮。
//!
//! 前台切换、全屏切换不等采样，立即重新调度（见 `cycle`）。所有判断都按时间而不是轮数，间隔变了结果不变。

use std::time::Duration;

use poise_core::model::Millis;

use super::Engine;

/// 采样间隔。
pub const TICK: Duration = Duration::from_secs(2);

/// 平静时的采样间隔。
const CALM_TICK: Duration = Duration::from_secs(4);

/// 最近一次变化（调整了程序、决定变了、前台切换）之后多久算平静。
const CALM_AFTER_MS: Millis = 20_000;

/// 没人在用电脑时的采样间隔。
const IDLE_TICK: Duration = Duration::from_secs(6);

/// 暂停着、界面也没显示时的采样间隔（不能太长：恢复后要等这一轮过去才回到平常的节奏）。
const PAUSED_TICK: Duration = Duration::from_secs(10);

/// 熄屏、待机时的采样间隔。
const STANDBY_TICK: Duration = Duration::from_secs(30);

/// 用户多久不操作算离开。
const AWAY_AFTER_MS: Millis = 2 * 60_000;

/// 切换模式在等预读原值时（第一次接管，见 `poise_hardware::Applied::Waiting`）的采样间隔：读好了尽快写。
const WAITING_TICK: Duration = Duration::from_millis(500);

impl Engine {
    /// 下一轮采样前等多久。
    pub fn next_tick_in(&self) -> Duration {
        let now = self.now();
        if self.mode_attempts.waiting(now) {
            return WAITING_TICK;
        }
        let gaming = self.view.as_ref().is_some_and(|v| v.game.is_some());
        let away = self.session.current(now).idle_ms >= AWAY_AFTER_MS || self.paused;
        interval(Moment {
            standby: self.standby.active(),
            ui: self.session.ui_visible(now),
            paused: self.paused,
            away,
            gaming,
            changing: now.saturating_sub(self.last_change_at) < CALM_AFTER_MS,
        })
    }
}

/// 决定采样快慢的情况。
#[derive(Debug, Clone, Copy, Default)]
struct Moment {
    /// 熄屏、待机着。
    standby: bool,
    /// 界面显示着（走势图、核心面板每 2 秒刷新）。
    ui: bool,
    /// 用户离开了（或 Poise 暂停）。
    away: bool,
    /// Poise 暂停着。
    paused: bool,
    gaming: bool,
    /// 刚有变化。
    changing: bool,
}

fn interval(m: Moment) -> Duration {
    if m.standby {
        STANDBY_TICK
    } else if m.paused && !m.ui {
        PAUSED_TICK
    } else if m.away && !m.gaming && !m.ui {
        IDLE_TICK
    } else if m.ui || m.changing {
        TICK
    } else {
        CALM_TICK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_frequent_only_around_changes_and_while_the_ui_is_open() {
        let calm = Moment::default();
        assert_eq!(interval(calm), CALM_TICK);
        // 游戏玩得正稳：和平时一样平静。
        let gaming = Moment {
            gaming: true,
            ..calm
        };
        assert_eq!(interval(gaming), CALM_TICK);
        // 刚开局、切换了窗口：勤一些，几秒内认出游戏、调整到位。
        assert_eq!(
            interval(Moment {
                changing: true,
                ..gaming
            }),
            TICK
        );
        assert_eq!(interval(Moment { ui: true, ..calm }), TICK);
        // 离开了：更慢；但游戏挂着（例如在等待队列）时照常。
        let away = Moment { away: true, ..calm };
        assert_eq!(interval(away), IDLE_TICK);
        assert_eq!(
            interval(Moment {
                gaming: true,
                ..away
            }),
            CALM_TICK
        );
        // 暂停着：很慢；界面开着时照常给界面数据。
        let paused = Moment {
            paused: true,
            away: true,
            ..calm
        };
        assert_eq!(interval(paused), PAUSED_TICK);
        assert_eq!(interval(Moment { ui: true, ..paused }), TICK);
        // 熄屏、待机：最慢，开着的界面也没人看。
        assert_eq!(
            interval(Moment {
                standby: true,
                ui: true,
                ..calm
            }),
            STANDBY_TICK
        );
    }
}
