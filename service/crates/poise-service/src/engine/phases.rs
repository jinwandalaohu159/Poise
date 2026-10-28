//! 一轮采样的分段计时：一轮太慢时，日志写明是哪一步慢，看日志就能定位。

use std::time::{Duration, Instant};

/// 一轮超过这个时间就记一条日志：采样期间引擎被锁住，前台切换和界面的请求要等它结束。
/// 正常一轮只要几毫秒（硬件读数、显卡占用都在各自的线程里）。
const SLOW_TICK: Duration = Duration::from_millis(150);

pub(super) struct Phases {
    started: Instant,
    last: Instant,
    slowest: (&'static str, Duration),
}

impl Phases {
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            slowest: ("", Duration::ZERO),
        }
    }

    /// 刚刚结束的一步。
    pub fn done(&mut self, phase: &'static str) {
        let now = Instant::now();
        let took = now - self.last;
        if took > self.slowest.1 {
            self.slowest = (phase, took);
        }
        self.last = now;
    }

    /// 一轮结束：太慢时记一条日志。
    pub fn finish(self) {
        let total = self.started.elapsed();
        if total > SLOW_TICK {
            let (phase, took) = self.slowest;
            log::info!(
                "本轮采样耗时 {} ms，最慢的一步：{phase} {} ms",
                total.as_millis(),
                took.as_millis()
            );
        }
    }
}
