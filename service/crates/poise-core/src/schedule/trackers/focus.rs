//! 重点程序：正在玩的游戏，以及正被直接使用、持续需要处理器的程序（前台的编译、渲染、导出视频，
//! 临时全速、设为始终优先的程序）。不分游戏还是别的程序，按同一套规则：
//! - 重点程序按自己需要的线程逐级占用核心（见 [`super::Footprint`]）：先主域（大核）；不够时再加一个
//!   核心域（一组共用二级缓存的小核、另一个 CCD——同一个域里线程之间共享的数据不用跨缓存搬）；直到所有核心；
//! - 有重点程序时，其余没在用的程序让到副域（不降优先级、不节能），主域留给它。最后一个重点程序结束后
//!   再保持一会儿：切换窗口、两局游戏之间的间隙，不会让一大批程序来回挪动。
//!   只有游戏（正在玩、等待确认、用户标记的）立即这样做；别的重点程序（前台的编译、导出）要持续
//!   [`FocusRule::work_settle_ms`] 才算：一次十几二十秒的增量编译自己照样用大核，但不为它把所有程序挪走
//!   再挪回来（得到的好处很少，换来两次全局调整、冷掉的缓存）。
//!
//! 这里只判断谁是重点程序；它占用多少核心见 `footprint`。

use std::collections::{HashMap, HashSet};

use crate::model::{Millis, ProcessKey};

/// 什么样的程序算重点程序。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRule {
    /// 正在用的程序平滑后的负载（单核百分比）达到这么多，算在干重活。
    pub enter_core_pct: f32,
    /// 成为重点程序之后，负载降到这么多以下才不算（进出门槛不同，不来回切换）。
    pub leave_core_pct: f32,
    /// 负载要持续低于离开门槛这么久才不算：编译、导出中间有几秒在读写文件、链接，那几秒闲着不算结束
    /// （否则结束后重新进入又要等 [`FocusRule::enter_after_ms`]，整个程序的核心范围来回改两次）。
    pub leave_after_ms: Millis,
    /// 正在用、负载持续达到进入门槛这么久，才算重点程序：打开网页、点开聊天窗口这类几秒钟的忙不算
    /// （实测浏览器、聊天软件被误认成重点程序，十几个程序跟着挪到小核、二十秒后又挪回来）。
    pub enter_after_ms: Millis,
    /// 重点程序离开前台（切出去看一眼消息、通知弹窗抢走前台）后，这么久内负载还在就仍算重点程序
    /// （实测一个程序两分钟里在"用大核"和"不再用大核"之间来回四十多次）。
    pub away_grace_ms: Millis,
    /// 最后一个重点程序结束后，其余程序在小核上再保持多久。
    pub hold_ms: Millis,
    /// 不是游戏的重点程序持续这么久，才让其余程序为它挪开（见模块说明）。
    pub work_settle_ms: Millis,
}

/// 这一轮是不是重点程序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusState {
    No,
    Yes,
    /// 这一轮起不再是重点程序，以及为什么（日志用）。
    Ended(FocusEnd),
}

impl FocusState {
    pub fn is_focused(self) -> bool {
        self == Self::Yes
    }
}

/// 重点程序为什么结束了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEnd {
    /// 负载降下来了。
    LoadDropped,
    /// 离开前台太久。
    LeftForeground,
}

/// 一个重点程序：最近一次被直接使用是什么时候，负载从什么时候起低于离开门槛。
#[derive(Debug, Clone, Copy)]
struct Focused {
    since: Millis,
    last_used: Millis,
    light_since: Option<Millis>,
}

impl Focused {
    fn new(now: Millis) -> Self {
        Self {
            since: now,
            last_used: now,
            light_since: None,
        }
    }
}

/// 两次看到干重活之间隔了这么久（平时最长 6 秒一轮）：中间没看着它，重新起算。
const HEAVY_GAP_MS: Millis = 15_000;

#[derive(Debug, Default)]
pub struct FocusTracker {
    focused: HashMap<ProcessKey, Focused>,
    /// 正在用、负载达到进入门槛的程序，从什么时候开始（见 [`FocusRule::enter_after_ms`]），
    /// 以及最近一次看到它的时刻：中间有一阵没按这条规则看它（等待确认的游戏、不参与调度时），
    /// 之前的起点不再算数。
    heavy_since: HashMap<ProcessKey, (Millis, Millis)>,
    /// 最近一次有让其余程序挪开的重点程序（游戏，或持续够久的）的时刻。
    last_focus_at: Option<Millis>,
}

impl FocusTracker {
    /// 每轮对每个程序调用一次，返回它是不是重点程序。`game` 表示它是正在玩（或等待确认）的游戏：
    /// 不看负载，总是重点程序。`in_use` 表示它正被直接使用；`load` 是它平滑后的负载。
    pub fn observe(
        &mut self,
        root: ProcessKey,
        game: bool,
        in_use: bool,
        load: f32,
        rule: FocusRule,
        now: Millis,
    ) -> FocusState {
        let heavy = in_use && load >= rule.enter_core_pct;
        if !heavy {
            self.heavy_since.remove(&root);
        }
        let state = if game {
            let since = self.focused.get(&root).map_or(now, |f| f.since);
            self.focused.insert(
                root,
                Focused {
                    since,
                    ..Focused::new(now)
                },
            );
            FocusState::Yes
        } else if let Some(f) = self.focused.get_mut(&root) {
            if in_use {
                f.last_used = now;
            }
            if load >= rule.leave_core_pct {
                f.light_since = None;
            }
            let light_for = f.light_since.map_or(0, |since| now.saturating_sub(since));
            let ended = if load < rule.leave_core_pct && light_for >= rule.leave_after_ms {
                Some(FocusEnd::LoadDropped)
            } else if now.saturating_sub(f.last_used) > rule.away_grace_ms {
                Some(FocusEnd::LeftForeground)
            } else {
                None
            };
            if ended.is_some() {
                self.focused.remove(&root);
            } else if load < rule.leave_core_pct {
                f.light_since.get_or_insert(now);
            }
            ended.map_or(FocusState::Yes, FocusState::Ended)
        } else if heavy && self.heavy_for(root, now) >= rule.enter_after_ms {
            self.heavy_since.remove(&root);
            self.focused.insert(root, Focused::new(now));
            FocusState::Yes
        } else {
            FocusState::No
        };
        let settled = game
            || self
                .focused
                .get(&root)
                .is_some_and(|f| now.saturating_sub(f.since) >= rule.work_settle_ms);
        if state.is_focused() && settled {
            self.last_focus_at = Some(now);
        }
        state
    }

    /// 持续干重活多久了（这一轮也算上）。上次看到它是很久以前时从现在重新算。
    fn heavy_for(&mut self, root: ProcessKey, now: Millis) -> Millis {
        let entry = self.heavy_since.entry(root).or_insert((now, now));
        if now.saturating_sub(entry.1) > HEAVY_GAP_MS {
            entry.0 = now;
        }
        entry.1 = now;
        now.saturating_sub(entry.0)
    }

    /// 有重点程序，或最后一个结束还不到 `hold_ms`：其余没在用的程序留在小核。
    pub fn holding(&self, now: Millis, hold_ms: Millis) -> bool {
        self.last_focus_at
            .is_some_and(|t| now.saturating_sub(t) < hold_ms)
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.focused.retain(|k, _| alive.contains(k));
        self.heavy_since.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey { pid: 9, created: 1 };
    const RULE: FocusRule = FocusRule {
        enter_core_pct: 80.0,
        leave_core_pct: 40.0,
        leave_after_ms: 10_000,
        enter_after_ms: 8_000,
        away_grace_ms: 15_000,
        hold_ms: 20_000,
        work_settle_ms: 60_000,
    };

    #[test]
    fn games_and_busy_programs_in_use_are_focused_alike() {
        let mut f = FocusTracker::default();
        // 游戏不看负载，也不用等。
        assert!(f.observe(KEY, true, false, 0.0, RULE, 0).is_focused());
        let editor = ProcessKey { pid: 3, created: 1 };
        // 正在用、持续占用接近一个核心：编译、导出这类工作，持续一会儿才算。
        assert!(!f.observe(editor, false, true, 50.0, RULE, 0).is_focused());
        assert!(
            !f.observe(editor, false, true, 90.0, RULE, 2_000)
                .is_focused()
        );
        assert!(
            !f.observe(editor, false, true, 90.0, RULE, 8_000)
                .is_focused()
        );
        assert!(
            f.observe(editor, false, true, 90.0, RULE, 10_000)
                .is_focused()
        );
        // 门槛之间：保持。
        assert!(
            f.observe(editor, false, true, 50.0, RULE, 12_000)
                .is_focused()
        );
        // 闲了几秒（链接、读写文件）又忙起来：照样是重点程序。
        for t in [14_000, 16_000, 18_000] {
            assert!(f.observe(editor, false, true, 30.0, RULE, t).is_focused());
        }
        assert!(
            f.observe(editor, false, true, 90.0, RULE, 20_000)
                .is_focused()
        );
        // 持续闲下来才结束。
        assert!(
            f.observe(editor, false, true, 30.0, RULE, 22_000)
                .is_focused()
        );
        assert_eq!(
            f.observe(editor, false, true, 30.0, RULE, 32_000),
            FocusState::Ended(FocusEnd::LoadDropped)
        );
        // 只忙了二十来秒的编译：自己用了大核，但没让别的程序挪开（开头的游戏早已结束保持）。
        assert!(!f.holding(35_000, RULE.hold_ms));
    }

    #[test]
    fn long_work_moves_the_others_aside_and_holds_after_it_ends() {
        let mut f = FocusTracker::default();
        let render = ProcessKey { pid: 4, created: 1 };
        let mut t = 0;
        while t <= 80_000 {
            f.observe(render, false, true, 95.0, RULE, t);
            // 持续够久之前，别的程序不为它挪开。
            if t < RULE.enter_after_ms + RULE.work_settle_ms {
                assert!(!f.holding(t, RULE.hold_ms), "{t}");
            }
            t += 2_000;
        }
        assert!(f.holding(80_000, RULE.hold_ms));
        // 结束后保持一会儿。
        let mut t = 82_000;
        while !matches!(
            f.observe(render, false, true, 0.0, RULE, t),
            FocusState::Ended(_)
        ) {
            t += 2_000;
        }
        assert!(!f.holding(t + RULE.hold_ms, RULE.hold_ms));
    }

    #[test]
    fn a_few_busy_seconds_do_not_make_a_focus() {
        let mut f = FocusTracker::default();
        // 打开网页：忙了 6 秒，中间断了一下，又忙 6 秒。
        for t in [0, 2_000, 4_000, 6_000] {
            assert!(!f.observe(KEY, false, true, 120.0, RULE, t).is_focused());
        }
        assert!(!f.observe(KEY, false, true, 20.0, RULE, 8_000).is_focused());
        for t in [10_000, 12_000, 14_000, 16_000] {
            assert!(!f.observe(KEY, false, true, 120.0, RULE, t).is_focused());
        }
        // 切到后台再忙也不算（那是"让路"的事）。
        assert!(
            !f.observe(KEY, false, false, 500.0, RULE, 30_000)
                .is_focused()
        );
    }

    #[test]
    fn a_focus_survives_brief_trips_away_from_the_foreground() {
        let mut f = FocusTracker::default();
        for t in (0..=8_000).step_by(2_000) {
            f.observe(KEY, false, true, 200.0, RULE, t);
        }
        assert!(
            f.observe(KEY, false, true, 200.0, RULE, 10_000)
                .is_focused()
        );
        // 切出去看一眼消息，几秒后回来：一直是重点程序。
        assert!(
            f.observe(KEY, false, false, 200.0, RULE, 12_000)
                .is_focused()
        );
        assert!(
            f.observe(KEY, false, false, 200.0, RULE, 20_000)
                .is_focused()
        );
        assert!(
            f.observe(KEY, false, true, 200.0, RULE, 22_000)
                .is_focused()
        );
        // 离开太久：不再是。
        assert!(
            f.observe(KEY, false, false, 200.0, RULE, 30_000)
                .is_focused()
        );
        assert_eq!(
            f.observe(KEY, false, false, 200.0, RULE, 38_000),
            FocusState::Ended(FocusEnd::LeftForeground)
        );
    }
}
