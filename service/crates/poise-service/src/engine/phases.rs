//! 一轮采样的分段计时：一轮太慢时，日志写明是哪一步慢，以及其中真正用掉的处理器时间——
//! 两者相差很多说明是在排队等处理器（Poise 以低优先级运行，整机很忙时会让着别人），而不是这一步本身慢。
//! 每一步的耗时也交给调用方（见 [`TickCost`]），手动测量开销时用。

use std::time::{Duration, Instant};

use poise_platform::observe::system::thread_cpu_time;

/// 一轮锁住引擎超过这个时间就记一条日志：这期间前台切换和界面的请求要等它结束。
/// 正常一轮只要一两毫秒（进程快照在锁外采集，硬件读数、显卡占用、电源设置的读写都在各自的线程里）。
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
#[derive(Debug, Clone, Copy, Default)]
pub struct Spent {
    pub wall: Duration,
    pub cpu: Duration,
}

/// 一轮的耗时：总共，以及按顺序的每一步。
#[derive(Debug, Clone, Default)]
pub struct TickCost {
    pub total: Spent,
    pub steps: Vec<(&'static str, Spent)>,
}

/// 量一下 `f` 花的时间。
pub(super) fn measure<T>(f: impl FnOnce() -> T) -> (T, Spent) {
    let start = Mark::now();
    let out = f();
    (out, since(start, Mark::now()))
}

pub(super) struct Phases {
    started: Mark,
    last: Mark,
    steps: Vec<(&'static str, Spent)>,
    /// 在引擎锁外做的步骤一共花的时间。
    outside: Spent,
}

impl Phases {
    pub fn start() -> Self {
        let now = Mark::now();
        Self {
            started: now,
            last: now,
            steps: Vec::with_capacity(12),
            outside: Spent::default(),
        }
    }

    /// 在引擎锁外做的一步（采集进程，见 `sampler`）：算进这一轮的开销，不算进锁住引擎的时间。
    pub fn outside(&mut self, phase: &'static str, spent: Spent) {
        self.steps.push((phase, spent));
        self.outside.wall += spent.wall;
        self.outside.cpu += spent.cpu;
    }

    /// 刚刚结束的一步。
    pub fn done(&mut self, phase: &'static str) {
        let now = Mark::now();
        self.steps.push((phase, since(self.last, now)));
        self.last = now;
    }

    /// 一轮结束：锁住引擎太久时记一条日志。
    pub fn finish(self) -> TickCost {
        let locked = since(self.started, Mark::now());
        if locked.wall > SLOW_TICK
            && let Some((phase, step)) = self.steps.iter().max_by_key(|(_, s)| s.wall)
        {
            log::info!(
                "本轮锁住引擎 {} ms（处理器 {} ms），最慢的一步：{phase} {} ms（处理器 {} ms）",
                locked.wall.as_millis(),
                locked.cpu.as_millis(),
                step.wall.as_millis(),
                step.cpu.as_millis()
            );
        }
        TickCost {
            total: Spent {
                wall: locked.wall + self.outside.wall,
                cpu: locked.cpu + self.outside.cpu,
            },
            steps: self.steps,
        }
    }
}

fn since(from: Mark, to: Mark) -> Spent {
    Spent {
        wall: to.at - from.at,
        cpu: to.cpu.saturating_sub(from.cpu),
    }
}

/// 每隔 [`CostLog::EVERY`] 在日志里写一次 Poise 自己的开销：每轮平均用多少时间、多少处理器时间，最费时的
/// 是哪一步。装在用户电脑上之后，从日志就能看出 Poise 是不是真的不吃资源。
#[derive(Default)]
pub(super) struct CostLog {
    since: Option<Instant>,
    rounds: u32,
    total: Spent,
    steps: Vec<(&'static str, Duration)>,
}

impl CostLog {
    const EVERY: Duration = Duration::from_secs(30 * 60);

    pub fn add(&mut self, cost: &TickCost) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.rounds += 1;
        self.total.wall += cost.total.wall;
        self.total.cpu += cost.total.cpu;
        for (name, spent) in &cost.steps {
            match self.steps.iter_mut().find(|(n, _)| n == name) {
                Some((_, wall)) => *wall += spent.wall,
                None => self.steps.push((name, spent.wall)),
            }
        }
        if since.elapsed() >= Self::EVERY {
            log::info!("{}", self.summary());
            *self = Self::default();
        }
    }

    fn summary(&self) -> String {
        let avg = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(self.rounds.max(1));
        let slowest = self
            .steps
            .iter()
            .max_by_key(|(_, wall)| *wall)
            .map_or_else(String::new, |(name, wall)| {
                format!("，最费时的一步：{name} {:.1} ms", avg(*wall))
            });
        format!(
            "Poise 自身开销（最近 {} 分钟，{} 轮）：每轮平均 {:.1} ms（处理器 {:.1} ms）{slowest}",
            self.since.map_or(0, |s| s.elapsed().as_secs() / 60),
            self.rounds,
            avg(self.total.wall),
            avg(self.total.cpu)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_summary_averages_rounds_and_names_the_slowest_step() {
        let ms = Duration::from_millis;
        let round = |collect, schedule| TickCost {
            total: Spent {
                wall: ms(collect + schedule),
                cpu: ms(collect),
            },
            steps: vec![
                (
                    "采集进程",
                    Spent {
                        wall: ms(collect),
                        cpu: ms(collect),
                    },
                ),
                (
                    "调度",
                    Spent {
                        wall: ms(schedule),
                        cpu: Duration::ZERO,
                    },
                ),
            ],
        };
        let mut log = CostLog::default();
        log.add(&round(4, 1));
        log.add(&round(6, 1));
        let s = log.summary();
        assert!(s.contains("2 轮"), "{s}");
        assert!(s.contains("每轮平均 6.0 ms（处理器 5.0 ms）"), "{s}");
        assert!(s.contains("最费时的一步：采集进程 5.0 ms"), "{s}");
    }
}
