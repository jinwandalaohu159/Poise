//! 正在玩的游戏。

use std::collections::{HashMap, HashSet};

use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};
use crate::prefs::AppPreference;

/// 正在玩的游戏。
/// - 用户标记的游戏：一运行就算，取消标记后不再算；
/// - 自动识别的（全屏且在渲染）：要持续一段时间才确认，偶尔的渲染尖峰不算；
/// - 确认后切到后台依然算"正在玩"，直到进程退出。
#[derive(Debug, Default)]
pub struct GameTracker {
    /// 正在玩的游戏，以及是不是用户标记的。
    active: HashMap<ProcessKey, bool>,
    /// 像游戏、等待确认的进程，以及从什么时候开始连续像游戏。
    candidates: HashMap<ProcessKey, Millis>,
}

impl GameTracker {
    pub fn update(
        &mut self,
        snap: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        now: Millis,
        confirm_ms: Millis,
    ) {
        let mut seen = HashSet::new();
        for p in &snap.processes {
            let key = p.facts.key;
            let Some(c) = classes.get(&key.pid) else {
                continue;
            };
            seen.insert(key);
            let marked = c.is_game && c.preference == AppPreference::Game;
            if marked {
                self.active.insert(key, true);
            } else if self.active.get(&key) == Some(&true) {
                // 用户取消了标记。
                self.active.remove(&key);
            }
            if c.is_game && !marked {
                let since = *self.candidates.entry(key).or_insert(now);
                if now.saturating_sub(since) >= confirm_ms {
                    self.active.entry(key).or_insert(false);
                }
            } else {
                self.candidates.remove(&key);
            }
        }
        self.active.retain(|k, _| seen.contains(k));
        self.candidates.retain(|k, _| seen.contains(k));
    }

    pub fn is_active(&self, key: &ProcessKey) -> bool {
        self.active.contains_key(key)
    }

    pub fn any_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn active_keys(&self) -> impl Iterator<Item = &ProcessKey> {
        self.active.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessSample};
    use crate::schedule::trackers::test_classes as classes;

    /// 进程 10 的分类：是否像游戏、是否被用户标记。
    fn game_world(looks_like_game: bool, marked: bool) -> (Snapshot, HashMap<Pid, Classification>) {
        let mut c = classes(None);
        let game = c.get_mut(&10).unwrap();
        game.is_game = looks_like_game || marked;
        if marked {
            game.preference = AppPreference::Game;
        }
        let snap = Snapshot {
            processes: [10, 20]
                .map(|pid| ProcessSample {
                    facts: ProcessFacts {
                        key: ProcessKey { pid, created: 1 },
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .to_vec(),
            ..Default::default()
        };
        (snap, c)
    }

    const KEY: ProcessKey = ProcessKey {
        pid: 10,
        created: 1,
    };

    #[test]
    fn detected_games_need_to_last_before_they_count() {
        let mut g = GameTracker::default();
        let (snap, looks) = game_world(true, false);
        let (_, idle) = game_world(false, false);
        g.update(&snap, &looks, 0, 6_000);
        g.update(&snap, &looks, 4_000, 6_000);
        // 一次中断就重新计时。
        g.update(&snap, &idle, 5_000, 6_000);
        g.update(&snap, &looks, 8_000, 6_000);
        assert!(!g.is_active(&KEY));
        g.update(&snap, &looks, 14_000, 6_000);
        assert!(g.is_active(&KEY));
        // 确认后切到后台（不再像游戏）依然算，直到进程退出。
        g.update(&snap, &idle, 20_000, 6_000);
        assert!(g.is_active(&KEY));
        let gone = Snapshot::default();
        g.update(&gone, &idle, 22_000, 6_000);
        assert!(!g.any_active());
    }

    #[test]
    fn marked_games_count_at_once_and_stop_when_unmarked() {
        let mut g = GameTracker::default();
        let (snap, marked) = game_world(false, true);
        let (_, unmarked) = game_world(false, false);
        g.update(&snap, &marked, 0, 6_000);
        assert!(g.is_active(&KEY));
        g.update(&snap, &unmarked, 1_000, 6_000);
        assert!(!g.is_active(&KEY));
    }
}
