//! 重点程序：正在玩的游戏，以及正被直接使用、持续需要处理器的程序（前台的编译、渲染、导出视频，
//! 临时全速、设为始终优先的程序）。不分游戏还是别的程序，按同一套规则：
//! - 重点程序按自己需要的线程逐级占用核心（见 [`Footprint`]）：先主域（大核）；不够时再加一个核心域
//!   （一组共用二级缓存的小核、另一个 CCD——同一个域里线程之间共享的数据不用跨缓存搬）；直到所有核心；
//! - 有重点程序时，其余没在用的程序让到副域（不降优先级、不节能），主域留给它。最后一个重点程序结束后
//!   再保持一会儿：切换窗口、两局游戏之间的间隙，不会让一大批程序来回挪动。

use std::collections::{HashMap, HashSet};

use crate::action::CoreSet;
use crate::model::{Millis, ProcessKey};
use crate::schedule::CoreLayout;

/// 什么样的程序算重点程序。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRule {
    /// 正在用的程序平滑后的负载（单核百分比）达到这么多，算重点程序。
    pub enter_core_pct: f32,
    /// 成为重点程序之后，负载降到这么多以下才不算（进出门槛不同，不来回切换）。
    pub leave_core_pct: f32,
    /// 最后一个重点程序结束后，其余程序在小核上再保持多久。
    pub hold_ms: Millis,
}

#[derive(Debug, Default)]
pub struct FocusTracker {
    focused: HashSet<ProcessKey>,
    /// 最近一次有重点程序的时刻。
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
    ) -> bool {
        let threshold = if self.focused.contains(&root) {
            rule.leave_core_pct
        } else {
            rule.enter_core_pct
        };
        let focused = game || (in_use && load >= threshold);
        if focused {
            self.focused.insert(root);
            self.last_focus_at = Some(now);
        } else {
            self.focused.remove(&root);
        }
        focused
    }

    /// 有重点程序，或最后一个结束还不到 `hold_ms`：其余没在用的程序留在小核。
    pub fn holding(&self, now: Millis, hold_ms: Millis) -> bool {
        self.last_focus_at
            .is_some_and(|t| now.saturating_sub(t) < hold_ms)
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.focused.retain(|k| alive.contains(k));
    }
}

/// 重点程序占用的核心：按需要的线程逐级放开。放开看此刻和平滑后的负载里较高的那个（突然变忙立即放开，
/// 一次可以跨几级）；收回只看平滑后的负载，并且放开后至少保持一会儿——宁可宽裕，不来回切换。
#[derive(Debug, Default)]
pub struct Footprint {
    /// 已经放开的程序：放开到第几级（见 [`Footprint::tiers`]），以及最近一次放开的时刻。
    widened: HashMap<ProcessKey, (usize, Millis)>,
}

impl Footprint {
    /// 负载达到这一级能力的这个比例时放开到下一级。
    const WIDEN_AT: f32 = 0.8;
    /// 负载降到上一级能力的这个比例以下才收回。
    const NARROW_AT: f32 = 0.5;
    /// 放开后至少保持这么久才收回（加载画面、编译的一阵忙不会刚放开就收回）。
    const KEEP_WIDE_MS: Millis = 10_000;

    /// `smoothed` 是平滑后的负载，`instant` 是此刻的负载（单核百分比）。
    pub fn cores_for(
        &mut self,
        root: ProcessKey,
        smoothed: f32,
        instant: f32,
        layout: CoreLayout,
        now: Millis,
    ) -> CoreSet {
        let tiers = layout.tiers();
        let capacity = |tier: usize| tiers[tier].1 as f32 * 100.0;
        let (mut tier, mut since) = self.widened.get(&root).copied().unwrap_or((0, now));
        tier = tier.min(tiers.len() - 1);
        let demand = smoothed.max(instant);
        let mut widened = false;
        while tier + 1 < tiers.len() && demand >= capacity(tier) * Self::WIDEN_AT {
            tier += 1;
            widened = true;
        }
        if widened {
            since = now;
        } else if tier > 0
            && smoothed <= capacity(tier - 1) * Self::NARROW_AT
            && now.saturating_sub(since) >= Self::KEEP_WIDE_MS
        {
            tier -= 1;
        }
        if tier == 0 {
            self.widened.remove(&root);
        } else {
            self.widened.insert(root, (tier, since));
        }
        tiers[tier].0
    }

    /// 不再是重点程序：下次从大核开始。
    pub fn forget(&mut self, root: &ProcessKey) {
        self.widened.remove(root);
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.widened.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey { pid: 9, created: 1 };
    const RULE: FocusRule = FocusRule {
        enter_core_pct: 80.0,
        leave_core_pct: 40.0,
        hold_ms: 20_000,
    };
    fn layout() -> CoreLayout {
        CoreLayout::new(&[12, 4, 4])
    }

    #[test]
    fn games_and_busy_programs_in_use_are_focused_alike() {
        let mut f = FocusTracker::default();
        // 游戏不看负载。
        assert!(f.observe(KEY, true, false, 0.0, RULE, 0));
        let editor = ProcessKey { pid: 3, created: 1 };
        // 正在用、持续占用接近一个核心：编译、导出这类工作。
        assert!(!f.observe(editor, false, true, 50.0, RULE, 0));
        assert!(f.observe(editor, false, true, 90.0, RULE, 2_000));
        assert!(f.observe(editor, false, true, 50.0, RULE, 4_000));
        assert!(!f.observe(editor, false, true, 30.0, RULE, 6_000));
        // 没在用（切到后台）就不再是重点程序，哪怕还很忙（那是"让路"的事）。
        assert!(!f.observe(editor, false, false, 500.0, RULE, 8_000));
        // 最后一个重点程序结束后保持一会儿。
        assert!(f.holding(20_000, RULE.hold_ms));
        assert!(!f.holding(30_000, RULE.hold_ms));
    }

    #[test]
    fn the_footprint_widens_fast_and_narrows_slowly() {
        let mut fp = Footprint::default();
        let mut at = |smoothed, instant, now| fp.cores_for(KEY, smoothed, instant, layout(), now);
        // 只靠一两个主线程：固定在大核。
        assert_eq!(at(150.0, 200.0, 0), CoreSet::Domains(0b1));
        // 超过大核能力（1200%）的 80%：多用一组小核（共 1600%）。
        assert_eq!(at(1000.0, 1000.0, 2_000), CoreSet::Domains(0b11));
        // 此刻的负载突然很高：不等平滑，直接放开到所有核心。
        assert_eq!(at(1000.0, 1900.0, 4_000), CoreSet::All);
        // 负载降下来也要先保持一会儿，再逐级收回。
        assert_eq!(at(300.0, 300.0, 6_000), CoreSet::All);
        assert_eq!(at(300.0, 300.0, 14_000), CoreSet::Domains(0b11));
        assert_eq!(at(300.0, 300.0, 16_000), CoreSet::Domains(0b1));
        // 在门槛之间：保持不变。
        assert_eq!(at(1000.0, 1000.0, 18_000), CoreSet::Domains(0b11));
        assert_eq!(at(700.0, 700.0, 40_000), CoreSet::Domains(0b11));
    }

    #[test]
    fn the_tiers_follow_the_core_domains() {
        let one_group = CoreLayout::new(&[12, 8]);
        let mut fp = Footprint::default();
        assert_eq!(fp.cores_for(KEY, 1000.0, 0.0, one_group, 0), CoreSet::All);
        // 双 CCD：先一个 CCD，不够再放开。
        let two_ccds = CoreLayout::new(&[16, 16]);
        let mut fp = Footprint::default();
        assert_eq!(
            fp.cores_for(KEY, 600.0, 600.0, two_ccds, 0),
            CoreSet::Domains(0b1)
        );
        assert_eq!(
            fp.cores_for(KEY, 600.0, 1400.0, two_ccds, 2_000),
            CoreSet::All
        );
        let mut fp = Footprint::default();
        let uniform = CoreLayout::uniform(16);
        assert_eq!(fp.cores_for(KEY, 50.0, 0.0, uniform, 0), CoreSet::All);
    }
}
