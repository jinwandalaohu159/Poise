//! 后台忙碌：哪些程序（进程树）该让路。

use std::collections::HashSet;

use super::demand::Pressure;
use crate::model::{Millis, ProcessKey};

/// 后台忙碌的判断规则。负载是整棵进程树平滑后的负载（见 `TreeLoad`），短暂的活动本来就不会让它升高。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusyRule {
    /// 需求一般时，负载（单核百分比）达到这么多算在忙。
    pub core_pct: f32,
    /// 需求高时（玩游戏、接近温度上限）的门槛：更低，后台更早让路。
    pub core_pct_high: f32,
    /// 已经让路的程序，持续这么久没有需求后恢复。
    pub release_after_ms: Millis,
}

/// 后台忙碌：有需求时，在后台持续占用处理器的程序让路；让路后保持，直到它回到"正在用"
/// （前台、发声、游戏等）、退出，或者长时间没有需求（没人需要资源了，后台的工作应该全速完成）。
/// 不因为它一时闲下来就恢复，避免反复调整。按进程树（程序）判断。
#[derive(Debug, Default)]
pub struct BusyTracker {
    yielded: HashSet<ProcessKey>,
}

impl BusyTracker {
    /// 每轮对每个程序调用一次，返回它现在是否应该让路。
    /// `idle_for` 是已经连续多久没有需求（见 `DemandTracker::idle_for`）。
    pub fn observe(
        &mut self,
        root: ProcessKey,
        load: f32,
        in_use: bool,
        pressure: Pressure,
        idle_for: Millis,
        rule: BusyRule,
    ) -> bool {
        if in_use {
            self.yielded.remove(&root);
            return false;
        }
        if self.yielded.contains(&root) {
            if pressure == Pressure::None && idle_for >= rule.release_after_ms {
                self.yielded.remove(&root);
                return false;
            }
            return true;
        }
        let threshold = match pressure {
            Pressure::None => return false,
            Pressure::Normal => rule.core_pct,
            Pressure::High => rule.core_pct_high,
        };
        if load >= threshold {
            self.yielded.insert(root);
            return true;
        }
        false
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.yielded.retain(|k| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: ProcessKey = ProcessKey {
        pid: 10,
        created: 1,
    };

    const RULE: BusyRule = BusyRule {
        core_pct: 10.0,
        core_pct_high: 3.0,
        release_after_ms: 60_000,
    };

    #[test]
    fn busy_programs_yield_only_when_someone_needs_the_processor() {
        let mut b = BusyTracker::default();
        // 没人需要：再忙也不让路。
        assert!(!b.observe(ROOT, 80.0, false, Pressure::None, 0, RULE));
        // 有需求：达到门槛才让路，需求高时门槛更低。
        assert!(!b.observe(ROOT, 5.0, false, Pressure::Normal, 0, RULE));
        assert!(b.observe(ROOT, 5.0, false, Pressure::High, 0, RULE));
        // 闲下来也保持让路，不来回调整。
        assert!(b.observe(ROOT, 0.0, false, Pressure::Normal, 0, RULE));
        // 回到前台（正在用）立即恢复。
        assert!(!b.observe(ROOT, 50.0, true, Pressure::Normal, 0, RULE));
    }

    #[test]
    fn yielded_programs_come_back_when_nobody_needs_the_processor() {
        let mut b = BusyTracker::default();
        assert!(b.observe(ROOT, 50.0, false, Pressure::Normal, 0, RULE));
        assert!(b.observe(ROOT, 50.0, false, Pressure::None, 30_000, RULE));
        assert!(!b.observe(ROOT, 50.0, false, Pressure::None, 60_000, RULE));
    }
}
