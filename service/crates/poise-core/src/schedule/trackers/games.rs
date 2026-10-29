//! 正在玩的游戏。

use std::collections::{HashMap, HashSet};

use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};
use crate::prefs::AppPreference;

/// 正在玩的游戏。
/// - 用户标记的游戏：一运行就算，取消标记后不再算；
/// - 自动识别的（全屏且在渲染）：要持续一段时间才确认，偶尔的渲染尖峰不算；中途短暂不像游戏
///   （加载画面、切换场景时渲染降下来）不打断计时；
/// - 确认后切到后台依然算"正在玩"，直到进程退出。
///
/// 游戏会话：最后一个游戏退出后保持一段时间（见 [`GameTracker::in_session`]）。
/// 一局结束回到客户端、选英雄、再进下一局，中间不切回日常模式——每次切换散热模式都卡一下。
#[derive(Debug, Default)]
pub struct GameTracker {
    /// 正在玩的游戏，以及是不是用户标记的。
    active: HashMap<ProcessKey, bool>,
    /// 像游戏、等待确认的进程：从什么时候开始像游戏，最近一次像游戏是什么时候。
    candidates: HashMap<ProcessKey, (Millis, Millis)>,
    /// 最近一次有游戏在运行的时间。
    last_played_at: Option<Millis>,
}

impl GameTracker {
    /// 等待确认期间，不像游戏的间隙短于这么久不打断计时。
    const GAP_MS: Millis = 4_000;

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
                let (since, last) = self.candidates.entry(key).or_insert((now, now));
                *last = now;
                if now.saturating_sub(*since) >= confirm_ms {
                    self.active.entry(key).or_insert(false);
                }
            } else if self
                .candidates
                .get(&key)
                .is_some_and(|(_, last)| now.saturating_sub(*last) > Self::GAP_MS)
                || marked
            {
                self.candidates.remove(&key);
            }
        }
        self.active.retain(|k, _| seen.contains(k));
        self.candidates.retain(|k, _| seen.contains(k));
        if !self.active.is_empty() {
            self.last_played_at = Some(now);
        }
    }

    /// 是否在游戏会话中：有游戏在运行，或最后一个游戏退出还不到 `hold_ms`。
    pub fn in_session(&self, now: Millis, hold_ms: Millis) -> bool {
        self.any_active()
            || self
                .last_played_at
                .is_some_and(|t| now.saturating_sub(t) < hold_ms)
    }

    pub fn any_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn active_keys(&self) -> impl Iterator<Item = &ProcessKey> {
        self.active.keys()
    }
}

/// 游戏要不要固定在大核。大核够用时固定（避免主线程被放到小核）；游戏需要的线程超过大核能提供的
/// （重度多线程的游戏），放开到所有核心，让小核也能分担。进出用不同的门槛，不来回切换。
#[derive(Debug, Default)]
pub struct GameCores {
    wide: HashSet<ProcessKey>,
}

impl GameCores {
    /// 负载达到大核能力的这个比例时放开。
    const WIDEN_AT: f32 = 0.8;
    /// 放开后，负载降到这个比例以下才重新固定。
    const NARROW_AT: f32 = 0.5;

    /// `load` 是游戏整个进程树平滑后的负载（单核百分比），`performance_threads` 是大核的线程数。
    pub fn needs_all_cores(
        &mut self,
        game: ProcessKey,
        load: f32,
        performance_threads: u32,
    ) -> bool {
        if performance_threads == 0 {
            return false;
        }
        let capacity = performance_threads as f32 * 100.0;
        let wide = if self.wide.contains(&game) {
            load > capacity * Self::NARROW_AT
        } else {
            load >= capacity * Self::WIDEN_AT
        };
        if wide {
            self.wide.insert(game);
        } else {
            self.wide.remove(&game);
        }
        wide
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.wide.retain(|k| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessSample};
    use crate::schedule::trackers::test_classes as classes;

    fn is_active(g: &GameTracker) -> bool {
        g.active_keys().any(|k| *k == KEY)
    }

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
        // 长时间中断（超过 4 秒）就重新计时。
        g.update(&snap, &idle, 6_000, 6_000);
        g.update(&snap, &idle, 10_000, 6_000);
        g.update(&snap, &looks, 12_000, 6_000);
        g.update(&snap, &looks, 16_000, 6_000);
        assert!(!is_active(&g));
        g.update(&snap, &looks, 18_000, 6_000);
        assert!(is_active(&g));
        // 确认后切到后台（不再像游戏）依然算，直到进程退出。
        g.update(&snap, &idle, 20_000, 6_000);
        assert!(is_active(&g));
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
        assert!(is_active(&g));
        g.update(&snap, &unmarked, 1_000, 6_000);
        assert!(!is_active(&g));
    }

    #[test]
    fn the_session_outlasts_the_game_for_a_while() {
        let mut t = GameTracker::default();
        let (snap, c) = game_world(false, true);
        t.update(&snap, &c, 0, 6_000);
        assert!(t.in_session(0, 180_000));
        let (snap, c) = game_world(false, false);
        t.update(&snap, &c, 2_000, 6_000);
        assert!(!t.any_active());
        assert!(t.in_session(60_000, 180_000));
        assert!(!t.in_session(200_000, 180_000));
    }

    #[test]
    fn games_that_outgrow_the_performance_cores_may_use_every_core() {
        let mut c = GameCores::default();
        // 12 个大核线程：英雄联盟这类只靠一两个主线程的游戏固定在大核。
        assert!(!c.needs_all_cores(KEY, 150.0, 12));
        // 重度多线程：超过大核能力的 80% 放开。
        assert!(c.needs_all_cores(KEY, 1000.0, 12));
        // 降到 50% 以下才重新固定，不来回切换。
        assert!(c.needs_all_cores(KEY, 700.0, 12));
        assert!(!c.needs_all_cores(KEY, 500.0, 12));
        // 不知道大核数量：总是固定。
        assert!(!c.needs_all_cores(KEY, 5000.0, 0));
    }

    #[test]
    fn brief_dips_do_not_restart_the_confirmation() {
        let mut g = GameTracker::default();
        let (game, c_game) = game_world(true, false);
        let (_, c_dip) = game_world(false, false);
        g.update(&game, &c_game, 0, 6_000);
        // 加载画面：渲染降下来 2 秒。
        g.update(&game, &c_dip, 2_000, 6_000);
        g.update(&game, &c_game, 4_000, 6_000);
        g.update(&game, &c_game, 6_000, 6_000);
        assert!(is_active(&g));
        // 长时间不像游戏：重新计时。
        let mut g = GameTracker::default();
        g.update(&game, &c_game, 0, 6_000);
        g.update(&game, &c_dip, 10_000, 6_000);
        g.update(&game, &c_game, 12_000, 6_000);
        assert!(!is_active(&g));
    }
}
