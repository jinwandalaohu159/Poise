//! 切换模式的尝试：第一次接管在等预读（下一轮要尽快来，见 `cadence`），或者切换失败了（隔一阵再试，
//! 越来越久——不每轮重试、每轮记一条失败）。

use poise_core::model::Millis;
use poise_core::prefs::Mode;
use poise_hardware::Applied;

/// 切换失败后第一次重试前等多久；之后每次加倍，最长 [`RETRY_MAX_MS`]。
const RETRY_FIRST_MS: Millis = 10_000;
const RETRY_MAX_MS: Millis = 60_000;

/// 等预读时最多这么久按快的节奏采样；电源服务卡住、一直读不完时回到平常的节奏（读好了照样会写）。
const WAITING_FAST_FOR_MS: Millis = 30_000;

/// 切换模式的尝试：在等预读（下一轮要尽快来），或者失败了（隔一阵再试，不每轮重试、每轮记一条失败）。
#[derive(Default)]
pub(super) struct ModeAttempts {
    /// 从什么时候开始等预读。
    waiting_since: Option<Millis>,
    /// 连续失败的目标、次数与最近一次的时刻。
    failed: Option<((Mode, bool), u32, Millis)>,
}

impl ModeAttempts {
    /// 在等预读、刚开始等不久：下一轮要尽快来。
    pub(super) fn waiting(&self, now: Millis) -> bool {
        self.waiting_since
            .is_some_and(|since| now.saturating_sub(since) < WAITING_FAST_FOR_MS)
    }

    /// 已经是这个模式了。
    pub(super) fn settled(&mut self) {
        *self = Self::default();
    }

    /// 现在该不该试：上次失败的是同一个目标时，等够了才再试；目标变了立即试。
    pub(super) fn due(&self, wanted: (Mode, bool), now: Millis) -> bool {
        match self.failed {
            Some((target, count, at)) if target == wanted => {
                let wait = RETRY_FIRST_MS
                    .saturating_mul(1 << (count - 1).min(10))
                    .min(RETRY_MAX_MS);
                now.saturating_sub(at) >= wait
            }
            _ => true,
        }
    }

    pub(super) fn record<E>(
        &mut self,
        wanted: (Mode, bool),
        result: &Result<Applied, E>,
        now: Millis,
    ) {
        self.waiting_since = match result {
            Ok(Applied::Waiting) => self.waiting_since.or(Some(now)),
            _ => None,
        };
        self.failed = match (result, self.failed) {
            (Ok(_), _) => None,
            (Err(_), Some((target, count, _))) if target == wanted => {
                Some((wanted, count + 1, now))
            }
            (Err(_), _) => Some((wanted, 1, now)),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_switches_are_retried_less_and_less_often() {
        let mut a = ModeAttempts::default();
        let full = (Mode::Full, false);
        let failed: Result<Applied, ()> = Err(());
        assert!(a.due(full, 0));
        a.record(full, &failed, 0);
        assert!(!a.due(full, RETRY_FIRST_MS - 1));
        // 目标变了（例如游戏退出了）：立即试。
        assert!(a.due((Mode::Balanced, false), 1));
        assert!(a.due(full, RETRY_FIRST_MS));
        a.record(full, &failed, RETRY_FIRST_MS);
        assert!(!a.due(full, RETRY_FIRST_MS * 2));
        assert!(a.due(full, RETRY_FIRST_MS * 3));
        for _ in 0..20 {
            a.record(full, &failed, 0);
        }
        assert!(a.due(full, RETRY_MAX_MS));
        a.record(full, &Ok::<_, ()>(Applied::Waiting), 0);
        assert!(a.waiting(0));
        assert!(a.due(full, 0));
        // 一直读不完：过了开头一阵就不再按快的节奏，再记一次等待也不重新计时。
        a.record(full, &Ok::<_, ()>(Applied::Waiting), WAITING_FAST_FOR_MS);
        assert!(!a.waiting(WAITING_FAST_FOR_MS));
        a.record(full, &Ok::<_, ()>(Applied::Written), WAITING_FAST_FOR_MS);
        assert!(!a.waiting(WAITING_FAST_FOR_MS));
    }
}
