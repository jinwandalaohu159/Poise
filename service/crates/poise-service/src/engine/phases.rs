//! 一轮采样的分段计时：一轮太慢时，日志写明是哪一步慢，以及其中真正用掉的处理器时间——
//! 两者相差很多说明是在排队等处理器（Poise 以低优先级运行，整机很忙时会让着别人），而不是这一步本身慢。

use std::time::{Duration, Instant};

use poise_platform::observe::system::thread_cpu_time;

/// 一轮超过这个时间就记一条日志：采样期间引擎被锁住，前台切换和界面的请求要等它结束。
/// 正常一轮只要几毫秒（硬件读数、显卡占用都在各自的线程里）。
const SLOW_TICK: Duration = Duration::from_millis(150);

/// 某个时刻：经过的时间与本线程用掉的处理器时间。
#[derive(Clone, Copy)]
struct Mark {
    at: Instant,
    cpu: Duration,
}

impl Mark {
    fn now() -> Self {
        Self {
            at: Instant::now(),
            cpu: thread_cpu_time(),
        }
    }
}

/// 一步花的时间：经过的时间与其中用掉的处理器时间。
#[derive(Clone, Copy, Default)]
struct Spent {
    wall: Duration,
    cpu: Duration,
}

pub(super) struct Phases {
    started: Mark,
    last: Mark,
    slowest: (&'static str, Spent),
}

impl Phases {
    pub fn start() -> Self {
        let now = Mark::now();
        Self {
            started: now,
            last: now,
            slowest: ("", Spent::default()),
        }
    }

    /// 刚刚结束的一步。
    pub fn done(&mut self, phase: &'static str) {
        let now = Mark::now();
        let spent = since(self.last, now);
        if spent.wall > self.slowest.1.wall {
            self.slowest = (phase, spent);
        }
        self.last = now;
    }

    /// 一轮结束：太慢时记一条日志。
    pub fn finish(self) {
        let total = since(self.started, Mark::now());
        if total.wall > SLOW_TICK {
            let (phase, step) = self.slowest;
            log::info!(
                "本轮采样耗时 {} ms（处理器 {} ms），最慢的一步：{phase} {} ms（处理器 {} ms）",
                total.wall.as_millis(),
                total.cpu.as_millis(),
                step.wall.as_millis(),
                step.cpu.as_millis()
            );
        }
    }
}

fn since(from: Mark, to: Mark) -> Spent {
    Spent {
        wall: to.at - from.at,
        cpu: to.cpu.saturating_sub(from.cpu),
    }
}
