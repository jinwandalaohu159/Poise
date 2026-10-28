//! 调度需要跨时间记住的三件事：前台防抖、正在运行的游戏、临时全速。

use std::collections::{HashMap, HashSet};

use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};

/// 前台防抖：切到前台立即提速，切走后延迟一段时间再降级。
#[derive(Debug, Default)]
pub struct ForegroundTracker {
    current_root: Option<Pid>,
    lost_at: HashMap<Pid, Millis>,
}

impl ForegroundTracker {
    pub fn update(
        &mut self,
        now: Millis,
        classes: &HashMap<Pid, Classification>,
        debounce_ms: Millis,
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
        self.lost_at
            .retain(|_, t| now.saturating_sub(*t) < debounce_ms);
    }

    /// 这个进程树是否刚失去前台（仍在防抖期内）。
    pub fn recently_foreground(&self, root: Pid) -> bool {
        self.lost_at.contains_key(&root)
    }
}

/// 记住被识别为游戏的进程：游戏切到后台后依然算"正在玩"，直到进程退出。
#[derive(Debug, Default)]
pub struct GameTracker {
    active: HashSet<ProcessKey>,
}

impl GameTracker {
    pub fn update(&mut self, snap: &Snapshot, classes: &HashMap<Pid, Classification>) {
        let alive: HashSet<ProcessKey> = snap.processes.iter().map(|p| p.facts.key).collect();
        self.active.retain(|k| alive.contains(k));
        for p in &snap.processes {
            if classes.get(&p.facts.key.pid).is_some_and(|c| c.is_game) {
                self.active.insert(p.facts.key);
            }
        }
    }

    pub fn is_active(&self, key: &ProcessKey) -> bool {
        self.active.contains(key)
    }

    pub fn any_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn active_keys(&self) -> impl Iterator<Item = &ProcessKey> {
        self.active.iter()
    }
}

/// 用户在托盘里选择的"临时全速"，到期自动失效。作用于整个进程树。
#[derive(Debug, Default)]
pub struct TempBoosts {
    until: HashMap<ProcessKey, Millis>,
}

impl TempBoosts {
    pub fn add(&mut self, key: ProcessKey, until: Millis) {
        self.until.insert(key, until);
    }

    pub fn expire(&mut self, now: Millis) {
        self.until.retain(|_, t| *t > now);
    }

    pub fn is_boosted(&self, key: &ProcessKey, root: Pid) -> bool {
        self.until.keys().any(|k| k == key || k.pid == root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Role, ScheduleScope};
    use crate::prefs::AppPreference;

    fn classes(fg_root: Option<Pid>) -> HashMap<Pid, Classification> {
        let mut m = HashMap::new();
        for root in [10, 20] {
            m.insert(
                root,
                Classification {
                    role: Role::UserApp,
                    root_pid: root,
                    in_foreground_tree: fg_root == Some(root),
                    has_visible_window: true,
                    tree_has_window: false,
                    plays_audio: false,
                    recently_active: false,
                    preference: AppPreference::Auto,
                    is_game: false,
                    anti_cheat_present: false,
                    restartable_service: None,
                    schedule: ScheduleScope::App,
                },
            );
        }
        m
    }

    #[test]
    fn foreground_loss_is_debounced() {
        let mut t = ForegroundTracker::default();
        t.update(0, &classes(Some(10)), 5000);
        t.update(1000, &classes(Some(20)), 5000);
        assert!(t.recently_foreground(10));
        t.update(7000, &classes(Some(20)), 5000);
        assert!(!t.recently_foreground(10));
    }

    #[test]
    fn returning_to_foreground_clears_debounce() {
        let mut t = ForegroundTracker::default();
        t.update(0, &classes(Some(10)), 5000);
        t.update(1000, &classes(Some(20)), 5000);
        t.update(1500, &classes(Some(10)), 5000);
        assert!(!t.recently_foreground(10));
        assert!(t.recently_foreground(20));
    }

    #[test]
    fn boosts_expire_and_cover_the_tree() {
        let mut b = TempBoosts::default();
        b.add(
            ProcessKey {
                pid: 10,
                created: 1,
            },
            1000,
        );
        assert!(b.is_boosted(
            &ProcessKey {
                pid: 11,
                created: 2
            },
            10
        ));
        b.expire(1000);
        assert!(!b.is_boosted(
            &ProcessKey {
                pid: 10,
                created: 1
            },
            10
        ));
    }
}
