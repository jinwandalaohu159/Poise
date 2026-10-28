//! 前台保持。

use std::collections::HashMap;

use crate::classify::Classification;
use crate::model::{Millis, Pid};

/// 前台保持：切走后一段时间内（默认 5 分钟）仍算"正在用"，期间不会让路。
/// 在几个程序之间来回切换时，它们都不会被反复调整（调整后再切回来要重新调度，容易卡一下）。
/// 只保持最近用过的几个程序，避免用过的程序全都一直不让路。
#[derive(Debug, Default)]
pub struct ForegroundTracker {
    current_root: Option<Pid>,
    lost_at: HashMap<Pid, Millis>,
}

/// 前台保持的规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundHold {
    /// 失去前台后保持多久。
    pub ms: Millis,
    /// 最多同时保持几个程序（按失去前台的时间，保留最近的）。
    pub max_apps: usize,
}

impl ForegroundTracker {
    pub fn update(
        &mut self,
        now: Millis,
        classes: &HashMap<Pid, Classification>,
        hold: ForegroundHold,
    ) {
        let root = classes
            .values()
            .find(|c| c.in_foreground_tree)
            .map(|c| c.root_pid);
        if root != self.current_root {
            if let Some(old) = self.current_root {
                self.lost_at.insert(old, now);
            }
            if let Some(new) = root {
                self.lost_at.remove(&new);
            }
            self.current_root = root;
        }
        self.lost_at.retain(|_, t| now.saturating_sub(*t) < hold.ms);
        while self.lost_at.len() > hold.max_apps {
            let oldest = self
                .lost_at
                .iter()
                .min_by_key(|(_, t)| **t)
                .map(|(root, _)| *root);
            if let Some(root) = oldest {
                self.lost_at.remove(&root);
            }
        }
    }

    /// 这个进程树是否刚失去前台（仍在保持期内）。
    pub fn recently_foreground(&self, root: Pid) -> bool {
        self.lost_at.contains_key(&root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::trackers::test_classes as classes;

    const HOLD: ForegroundHold = ForegroundHold {
        ms: 300_000,
        max_apps: 3,
    };

    #[test]
    fn foreground_is_held_for_a_while_after_switching_away() {
        let mut t = ForegroundTracker::default();
        t.update(0, &classes(Some(10)), HOLD);
        t.update(1000, &classes(Some(20)), HOLD);
        t.update(200_000, &classes(Some(20)), HOLD);
        assert!(t.recently_foreground(10));
        t.update(301_000, &classes(Some(20)), HOLD);
        assert!(!t.recently_foreground(10));
    }

    #[test]
    fn returning_to_foreground_clears_the_hold() {
        let mut t = ForegroundTracker::default();
        t.update(0, &classes(Some(10)), HOLD);
        t.update(1000, &classes(Some(20)), HOLD);
        t.update(1500, &classes(Some(10)), HOLD);
        assert!(!t.recently_foreground(10));
        assert!(t.recently_foreground(20));
    }

    #[test]
    fn only_the_most_recent_apps_are_held() {
        let one = ForegroundHold {
            ms: 300_000,
            max_apps: 1,
        };
        let mut t = ForegroundTracker::default();
        t.update(0, &classes(Some(10)), one);
        t.update(1000, &classes(Some(20)), one);
        t.update(2000, &classes(None), one);
        assert!(!t.recently_foreground(10));
        assert!(t.recently_foreground(20));
    }
}
