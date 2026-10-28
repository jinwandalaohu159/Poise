//! 后台忙碌。

use std::collections::{HashMap, HashSet};

use crate::model::{Millis, ProcessKey};

/// 后台忙碌的判断规则。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusyRule {
    /// 单核占用达到这个百分比算在忙。
    pub core_pct: f32,
    /// 要连续忙这么久才让路（短暂的活动不值得调整）。
    pub after_ms: Millis,
}

/// 后台忙碌：在后台持续占用处理器的进程才让路；让路后一直保持，直到它回到"正在用"
/// （前台、发声、游戏等）或退出。不因为一时闲下来就恢复，避免反复调整。
#[derive(Debug, Default)]
pub struct BusyTracker {
    /// 正在忙、还没到让路时间的进程，以及从什么时候开始连续忙。
    since: HashMap<ProcessKey, Millis>,
    yielded: HashSet<ProcessKey>,
}

impl BusyTracker {
    /// 每个周期对每个进程调用一次，返回它现在是否应该让路。
    pub fn observe(
        &mut self,
        key: ProcessKey,
        core_pct: f32,
        in_use: bool,
        now: Millis,
        rule: BusyRule,
    ) -> bool {
        if in_use {
            self.since.remove(&key);
            self.yielded.remove(&key);
            return false;
        }
        if self.yielded.contains(&key) {
            return true;
        }
        if core_pct < rule.core_pct {
            self.since.remove(&key);
            return false;
        }
        let since = *self.since.entry(key).or_insert(now);
        if now.saturating_sub(since) >= rule.after_ms {
            self.since.remove(&key);
            self.yielded.insert(key);
            return true;
        }
        false
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.since.retain(|k, _| alive.contains(k));
        self.yielded.retain(|k| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey {
        pid: 10,
        created: 1,
    };

    const BUSY: BusyRule = BusyRule {
        core_pct: 10.0,
        after_ms: 10_000,
    };

    #[test]
    fn only_sustained_background_work_yields_and_then_stays() {
        let mut b = BusyTracker::default();
        // 短暂的活动不让路。
        assert!(!b.observe(KEY, 50.0, false, 0, BUSY));
        assert!(!b.observe(KEY, 1.0, false, 4_000, BUSY));
        assert!(!b.observe(KEY, 50.0, false, 6_000, BUSY));
        assert!(!b.observe(KEY, 50.0, false, 14_000, BUSY));
        assert!(b.observe(KEY, 50.0, false, 16_000, BUSY));
        // 闲下来也保持让路，不来回调整。
        assert!(b.observe(KEY, 0.0, false, 20_000, BUSY));
        // 回到前台（正在用）立即恢复。
        assert!(!b.observe(KEY, 50.0, true, 22_000, BUSY));
        assert!(!b.observe(KEY, 50.0, false, 24_000, BUSY));
    }
}
