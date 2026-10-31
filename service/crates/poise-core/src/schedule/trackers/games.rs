//! 正在玩的游戏。

use std::collections::{HashMap, HashSet};

use crate::activity::ActivityTracker;
use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};
use crate::prefs::AppPreference;

/// 正在玩的游戏。
/// - 用户标记的游戏：一运行就算，取消标记后不再算；
/// - 自动识别的（全屏且在渲染）：要持续一段时间才确认，偶尔的渲染尖峰不算；中途短暂不像游戏
///   （加载画面、切换场景时渲染降下来）不打断计时；
/// - 确认后切到后台依然算"正在玩"（切出去看一眼、等待排队）；在后台闲着很久（挂着不玩、最小化）
///   就先放一边（见 [`GameTracker::PARK_AFTER_MS`]）：不再保持游戏模式、不再让别的程序为它让路，
///   回到前台立即恢复，不用重新确认。
///
/// 游戏会话：最后一个游戏退出后保持一段时间（见 [`GameTracker::in_session`]）。
/// 一局结束回到客户端、选英雄、再进下一局，中间不切回日常模式——每次切换散热模式都卡一下。
#[derive(Debug, Default)]
pub struct GameTracker {
    /// 正在玩的游戏，以及是不是用户标记的。
    active: HashMap<ProcessKey, bool>,
    /// 像游戏、等待确认的进程：从什么时候开始像游戏，最近一次像游戏是什么时候。
    candidates: HashMap<ProcessKey, (Millis, Millis)>,
    /// 正在玩的游戏从什么时候起不在前台。
    away_since: HashMap<ProcessKey, Millis>,
    /// 确认过、但在后台闲着放到一边的游戏，以及是不是用户标记的。
    parked: HashMap<ProcessKey, bool>,
    /// 最近一次有游戏在运行的时间。
    last_played_at: Option<Millis>,
}

impl GameTracker {
    /// 等待确认期间，不像游戏的间隙短于这么久不打断计时。
    const GAP_MS: Millis = 4_000;
    /// 游戏不在前台这么久、并且闲着（见 [`Self::PARK_BELOW_PCT`]），就先放到一边。
    const PARK_AFTER_MS: Millis = 3 * 60_000;
    /// 游戏进程的处理器占用（单核百分比）低于这个算闲着：最小化、暂停的游戏几乎不用处理器，
    /// 在后台照样跑着（例如挂机）的不算。
    const PARK_BELOW_PCT: f32 = 10.0;
    /// 自动识别的游戏要用到这么多处理器（单核百分比）才确认：全屏看高分辨率视频时显卡也忙，
    /// 但视频播放器几乎不用处理器；游戏的主线程总要用掉一大块。
    const CONFIRM_MIN_PCT: f32 = 20.0;
    /// 等待确认最多这么久（见 [`GameTracker::pending_keys`]）。
    const MAX_PENDING_MS: Millis = 60_000;

    pub fn update(
        &mut self,
        snap: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        activity: &ActivityTracker,
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
            // 只认程序的主进程：标为游戏的程序树里的子进程（崩溃处理、辅助进程）跟着主进程走，
            // 不各自算一个游戏（算了就会独立出去，各自成为一个"正在玩"的程序）。
            let marked = c.is_game && c.preference == AppPreference::Game && c.root_pid == key.pid;
            if marked && !self.parked.contains_key(&key) {
                self.active.insert(key, true);
            } else if !marked && self.active.get(&key) == Some(&true) {
                // 用户取消了标记。
                self.active.remove(&key);
            }
            // 标为游戏的程序里的子进程（继承了"游戏"标记）也不另算候选：否则它自己用的处理器一多就被确认、
            // 独立出去，带着继承来的标记被提到"高于正常"（例如客户端的渲染进程）。
            let inherited = c.preference == AppPreference::Game && c.root_pid != key.pid;
            if c.is_game && !marked && !inherited {
                let (since, last) = self.candidates.entry(key).or_insert((now, now));
                *last = now;
                if now.saturating_sub(*since) >= confirm_ms
                    && activity.usage(&key).core_pct >= Self::CONFIRM_MIN_PCT
                {
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
        self.parked.retain(|k, _| seen.contains(k));
        self.park_or_resume(classes, activity, now);
        if !self.active.is_empty() {
            self.last_played_at = Some(now);
        }
    }

    /// 在后台闲了很久的游戏放到一边；放到一边的回到前台立即恢复。
    fn park_or_resume(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        activity: &ActivityTracker,
        now: Millis,
    ) {
        let in_front = |k: &ProcessKey| classes.get(&k.pid).is_some_and(|c| c.in_foreground_tree);
        let back: Vec<ProcessKey> = self.parked.keys().copied().filter(in_front).collect();
        for key in back {
            if let Some(marked) = self.parked.remove(&key) {
                self.active.insert(key, marked);
            }
        }
        let mut idle = Vec::new();
        for key in self.active.keys() {
            if in_front(key) {
                self.away_since.remove(key);
                continue;
            }
            let since = *self.away_since.entry(*key).or_insert(now);
            if now.saturating_sub(since) >= Self::PARK_AFTER_MS
                && activity.usage(key).core_pct < Self::PARK_BELOW_PCT
            {
                idle.push(*key);
            }
        }
        for key in idle {
            if let Some(marked) = self.active.remove(&key) {
                self.away_since.remove(&key);
                self.parked.insert(key, marked);
            }
        }
        let active = &self.active;
        self.away_since.retain(|k, _| active.contains_key(k));
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

    /// 像游戏、还在等待确认的进程。放核心这种随时可撤的调整不必等确认（游戏一开就在大核上），
    /// 切换模式、让其他程序让路要等确认。等了 [`Self::MAX_PENDING_MS`] 还没确认的（全屏看视频、
    /// 可视化程序：显卡在忙，处理器用得不多）不再当作等待确认的游戏。
    pub fn pending_keys(&self, now: Millis) -> impl Iterator<Item = &ProcessKey> {
        self.candidates
            .iter()
            .filter(move |(k, (since, _))| {
                !self.active.contains_key(k) && now.saturating_sub(*since) <= Self::MAX_PENDING_MS
            })
            .map(|(k, _)| k)
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

    /// 进程 10 用掉半个核心（游戏的主线程总在忙）。
    fn busy() -> ActivityTracker {
        let mut a = ActivityTracker::default();
        for (at, cpu_time) in [(0, 0), (2_000, 10_000_000)] {
            a.update(&Snapshot {
                at,
                processes: vec![ProcessSample {
                    facts: ProcessFacts {
                        key: KEY,
                        ..Default::default()
                    },
                    metrics: crate::model::ProcessMetrics {
                        cpu_time,
                        ..Default::default()
                    },
                }],
                ..Default::default()
            });
        }
        a
    }

    #[test]
    fn fullscreen_video_without_cpu_work_is_not_confirmed_as_a_game() {
        let mut g = GameTracker::default();
        let (snap, looks) = game_world(true, false);
        for at in (0..=30_000).step_by(2_000) {
            g.update(&snap, &looks, &ActivityTracker::default(), at, 6_000);
        }
        assert!(!is_active(&g));
        assert!(g.pending_keys(30_000).any(|k| *k == KEY));
        // 一直确认不了：不再当作等待确认的游戏（不一直占着大核）。
        for at in (32_000..=70_000).step_by(2_000) {
            g.update(&snap, &looks, &ActivityTracker::default(), at, 6_000);
        }
        assert_eq!(g.pending_keys(70_000).count(), 0);
    }

    #[test]
    fn detected_games_need_to_last_before_they_count() {
        let busy = busy();
        let mut g = GameTracker::default();
        let (snap, looks) = game_world(true, false);
        let (_, idle) = game_world(false, false);
        g.update(&snap, &looks, &busy, 0, 6_000);
        g.update(&snap, &looks, &busy, 4_000, 6_000);
        // 长时间中断（超过 4 秒）就重新计时。
        g.update(&snap, &idle, &busy, 6_000, 6_000);
        g.update(&snap, &idle, &busy, 10_000, 6_000);
        g.update(&snap, &looks, &busy, 12_000, 6_000);
        g.update(&snap, &looks, &busy, 16_000, 6_000);
        assert!(!is_active(&g));
        assert!(g.pending_keys(16_000).any(|k| *k == KEY));
        g.update(&snap, &looks, &busy, 18_000, 6_000);
        assert!(is_active(&g));
        assert_eq!(g.pending_keys(18_000).count(), 0);
        // 确认后切到后台（不再像游戏）依然算，直到进程退出。
        g.update(&snap, &idle, &busy, 20_000, 6_000);
        assert!(is_active(&g));
        let gone = Snapshot::default();
        g.update(&gone, &idle, &busy, 22_000, 6_000);
        assert!(!g.any_active());
    }

    #[test]
    fn marked_games_count_at_once_and_stop_when_unmarked() {
        let mut g = GameTracker::default();
        let (snap, marked) = game_world(false, true);
        let (_, unmarked) = game_world(false, false);
        g.update(&snap, &marked, &ActivityTracker::default(), 0, 6_000);
        assert!(is_active(&g));
        g.update(&snap, &unmarked, &ActivityTracker::default(), 1_000, 6_000);
        assert!(!is_active(&g));
    }

    #[test]
    fn children_of_a_marked_game_are_not_games_of_their_own() {
        let busy = busy();
        let mut g = GameTracker::default();
        // 进程 10 是标为游戏的程序（20）里的子进程：继承了标记，又像游戏、又在忙。
        let (snap, mut c) = game_world(true, true);
        c.get_mut(&10).unwrap().root_pid = 20;
        c.get_mut(&20).unwrap().preference = AppPreference::Game;
        for at in (0..=30_000).step_by(2_000) {
            g.update(&snap, &c, &busy, at, 6_000);
        }
        assert!(!is_active(&g));
        assert_eq!(g.pending_keys(30_000).count(), 0);
    }

    #[test]
    fn the_session_outlasts_the_game_for_a_while() {
        let mut t = GameTracker::default();
        let (snap, c) = game_world(false, true);
        t.update(&snap, &c, &ActivityTracker::default(), 0, 6_000);
        assert!(t.in_session(0, 180_000));
        let (snap, c) = game_world(false, false);
        t.update(&snap, &c, &ActivityTracker::default(), 2_000, 6_000);
        assert!(!t.any_active());
        assert!(t.in_session(60_000, 180_000));
        assert!(!t.in_session(200_000, 180_000));
    }

    #[test]
    fn brief_dips_do_not_restart_the_confirmation() {
        let busy = busy();
        let mut g = GameTracker::default();
        let (game, c_game) = game_world(true, false);
        let (_, c_dip) = game_world(false, false);
        g.update(&game, &c_game, &busy, 0, 6_000);
        // 加载画面：渲染降下来 2 秒。
        g.update(&game, &c_dip, &busy, 2_000, 6_000);
        g.update(&game, &c_game, &busy, 4_000, 6_000);
        g.update(&game, &c_game, &busy, 6_000, 6_000);
        assert!(is_active(&g));
        // 长时间不像游戏：重新计时。
        let mut g = GameTracker::default();
        g.update(&game, &c_game, &busy, 0, 6_000);
        g.update(&game, &c_dip, &busy, 10_000, 6_000);
        g.update(&game, &c_game, &busy, 12_000, 6_000);
        assert!(!is_active(&g));
    }
}
