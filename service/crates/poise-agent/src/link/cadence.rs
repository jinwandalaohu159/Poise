//! 和服务打交道的节奏：多久采集一次桌面、没变化时多久保活一次、连不上时多久重连一次。
//! 只做计算，不碰管道和桌面，单独测试。

use std::time::{Duration, Instant};

/// 会话心跳：每 3 秒采集一次（音频会话只在心跳里重新枚举，开始发声的后台程序最迟 3 秒内被照顾到），
/// 有变化才上报；前台变化时立即上报，不等心跳。
pub const HEARTBEAT: Duration = Duration::from_secs(3);
/// 状态没变时也大约隔这么久上报一次：服务据此知道托盘代理还在（20 秒没有上报就视为看不到桌面），
/// 也顺便更新"多久没操作"。
pub const KEEPALIVE: Duration = Duration::from_secs(6);
/// 用户这么久没操作（离开了、锁着屏）时，心跳放慢到保活间隔：人不在时不必每 3 秒采集一次桌面。
/// 和服务认定"离开"的时间一致。
pub const AWAY_AFTER_MS: u64 = 2 * 60_000;
/// 这个会话不是正在用的那个（快速切换用户后切走了、远程连接断开了）时多久看一次：不报也不采集，
/// 切回来时有会话通知立即上报，这里只是兜底。
pub const INACTIVE_HEARTBEAT: Duration = Duration::from_secs(30);
/// 托盘图标的状态每几次心跳刷新一次（和心跳一起醒来，不另外醒）。
pub const STATUS_EVERY_BEATS: u32 = 3;

/// 这次心跳之后隔多久再来。
pub fn heartbeat(idle_ms: u64) -> Duration {
    if idle_ms >= AWAY_AFTER_MS {
        KEEPALIVE
    } else {
        HEARTBEAT
    }
}

/// 桌面没变化时，这次要不要也报（保活）：再等一次心跳，离上次上报就明显超过保活间隔了。
/// 心跳总会晚一点点到，所以按半个心跳留余量——按整个心跳算的话每次心跳都会报。
pub fn keepalive_due(since_last: Duration, heartbeat: Duration) -> bool {
    since_last + heartbeat / 2 >= KEEPALIVE
}

/// 连不上服务时每 3 秒重连一次（和 0.7.0 一样，不越拉越长）：服务重启、更新后尽快接上——连不上的这段时间
/// 服务一直暂停着。一次失败的重连只是打开一次管道（先连上才采集桌面），代价很小。
/// 醒来、解锁、切回这个用户时立即再试（[`Reconnect::now`]）。
#[derive(Debug, Default)]
pub struct Reconnect {
    last: Option<Instant>,
}

impl Reconnect {
    const EVERY: Duration = Duration::from_secs(3);

    /// 现在可以再试一次。
    pub fn due(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|at| now.saturating_duration_since(at) >= Self::EVERY)
    }

    pub fn attempted(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// 下次不用等，立即再试。
    pub fn now(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_desktops_are_reported_every_other_heartbeat() {
        // 心跳总会晚到一点点：3.01 秒时不报，6.02 秒时报。
        let late = Duration::from_millis(10);
        assert!(!keepalive_due(HEARTBEAT + late, HEARTBEAT));
        assert!(keepalive_due(2 * HEARTBEAT + 2 * late, HEARTBEAT));
        // 人离开后心跳就是保活间隔：每次都报。
        let away = heartbeat(AWAY_AFTER_MS);
        assert_eq!(away, KEEPALIVE);
        assert!(keepalive_due(away + late, away));
    }

    #[test]
    fn reconnects_every_few_seconds_and_at_once_when_asked() {
        let start = Instant::now();
        let mut r = Reconnect::default();
        assert!(r.due(start));
        for i in 0..10 {
            let at = start + Reconnect::EVERY * i;
            r.attempted(at);
            assert!(!r.due(at + Reconnect::EVERY - Duration::from_millis(1)));
            assert!(r.due(at + Reconnect::EVERY));
        }
        r.attempted(start);
        r.now();
        assert!(r.due(start));
    }
}
