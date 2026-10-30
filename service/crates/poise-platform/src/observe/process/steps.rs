//! 采集一次的分步计时：超过 [`SLOW_TAKE`] 时日志里写明每一步用了多久（多半是在等系统服务回应，而不是在算）。

use std::time::{Duration, Instant};

/// 采集一次超过这么久时记日志。
const SLOW_TAKE: Duration = Duration::from_millis(100);

/// 采集的各步用时。
pub(super) struct Steps {
    started: Instant,
    last: Instant,
    done: Vec<(&'static str, Duration)>,
}

impl Steps {
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            done: Vec::new(),
        }
    }

    pub fn done(&mut self, step: &'static str) {
        let now = Instant::now();
        self.done.push((step, now - self.last));
        self.last = now;
    }

    pub fn log_if_slow(&self) {
        let total = self.started.elapsed();
        if total < SLOW_TAKE {
            return;
        }
        let parts: Vec<String> = self
            .done
            .iter()
            .map(|(step, d)| format!("{step} {}", d.as_millis()))
            .collect();
        log::info!(
            "采集进程用了 {} ms（毫秒）：{}",
            total.as_millis(),
            parts.join("，")
        );
    }
}
