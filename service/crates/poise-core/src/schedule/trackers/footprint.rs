//! 重点程序占用多少核心（谁是重点程序见 `focus`）。

use crate::action::CoreSet;
use crate::fast_hash::{HashMap, HashSet};
use crate::model::{Millis, ProcessKey};
use crate::schedule::CoreLayout;

/// 重点程序的负载（单核百分比）。
#[derive(Debug, Clone, Copy, Default)]
pub struct FocusLoad {
    /// 按半分钟平滑。
    pub smoothed: f32,
    /// 按几秒平滑。
    pub recent: f32,
    /// 连续两轮都达到的。
    pub sustained: f32,
}

/// 重点程序占用的核心：按需要的线程逐级放开，每轮最多放开一级。放开看平滑后的、快平滑的、连续两轮都达到的
/// 负载里最高的那个（见 [`FocusLoad`]）：真的变忙几秒内就放开，加载画面、自动存档这样只有一轮的尖峰不放开（放开要对游戏的
/// 每个进程改核心范围，副域上的程序也要跟着挪）。收回只看平滑后的负载，并且放开后至少保持一会儿——
/// 宁可宽裕，不来回切换。
///
/// 同时有几个重点程序（两个游戏、一边玩一边导出视频）时，它们都从主域开始、挤在同一组核心上：
/// 每个程序看的是加上不比它重的其他重点程序（上一轮的平滑负载）之后的总负载。重的先放开、让出位置，
/// 只靠一两个线程的轻程序留在主域；差不多重的一起放开。
#[derive(Debug, Default)]
pub struct Footprint {
    programs: HashMap<ProcessKey, Footing>,
}

#[derive(Debug, Clone, Copy)]
struct Footing {
    /// 放开到第几级（见 [`CoreLayout::tiers`]）。
    tier: usize,
    /// 最近一次放开的时刻。
    since: Millis,
    /// 最近一次的平滑负载（别的重点程序按它算挤在一起的总负载）。
    load: f32,
}

impl Footprint {
    /// 负载达到这一级能力的这个比例时放开到下一级。
    const WIDEN_AT: f32 = 0.8;
    /// 负载降到上一级能力的这个比例以下才收回。
    const NARROW_AT: f32 = 0.5;
    /// 放开后至少保持这么久才收回（加载画面、编译的一阵忙不会刚放开就收回）。
    const KEEP_WIDE_MS: Millis = 10_000;

    pub fn cores_for(
        &mut self,
        root: ProcessKey,
        load: FocusLoad,
        layout: CoreLayout,
        now: Millis,
    ) -> CoreSet {
        let tiers = layout.tiers();
        let others: f32 = self
            .programs
            .iter()
            .filter(|(k, f)| **k != root && f.load <= load.smoothed)
            .map(|(_, f)| f.load)
            .sum();
        let f = self.programs.entry(root).or_insert(Footing {
            tier: 0,
            since: now,
            load: 0.0,
        });
        f.tier = f.tier.min(tiers.len() - 1);
        f.load = load.smoothed;
        let demand = load.smoothed.max(load.recent).max(load.sustained) + others;
        if f.tier + 1 < tiers.len() && demand >= tiers[f.tier].1 * Self::WIDEN_AT {
            f.tier += 1;
            f.since = now;
        } else if f.tier > 0
            && load.smoothed + others <= tiers[f.tier - 1].1 * Self::NARROW_AT
            && now.saturating_sub(f.since) >= Self::KEEP_WIDE_MS
        {
            f.tier -= 1;
        }
        tiers[f.tier].0
    }

    /// 不再是重点程序：下次从主域开始。
    pub fn forget(&mut self, root: &ProcessKey) {
        self.programs.remove(root);
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.programs.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_focused_programs_widen_together() {
        let mut fp = Footprint::default();
        let other = ProcessKey {
            pid: 10,
            created: 1,
        };
        // 各用 500%：单独一个放得下大核（1200% 的 80% 是 960%），两个加起来就放不下了。
        assert_eq!(
            fp.cores_for(KEY, load(500.0, 500.0), layout(), 0),
            CoreSet::Domains(0b1)
        );
        assert_eq!(
            fp.cores_for(other, load(500.0, 500.0), layout(), 0),
            CoreSet::Domains(0b11)
        );
        assert_eq!(
            fp.cores_for(KEY, load(500.0, 500.0), layout(), 2_000),
            CoreSet::Domains(0b11)
        );
        // 轻的那个不因为重的而放开。
        let light = ProcessKey {
            pid: 11,
            created: 1,
        };
        assert_eq!(
            fp.cores_for(light, load(100.0, 100.0), layout(), 2_000),
            CoreSet::Domains(0b1)
        );
        fp.forget(&light);
        // 另一个结束了：负载回落后照常收回。
        fp.forget(&other);
        assert_eq!(
            fp.cores_for(KEY, load(200.0, 200.0), layout(), 20_000),
            CoreSet::Domains(0b1)
        );
    }

    const KEY: ProcessKey = ProcessKey { pid: 9, created: 1 };

    fn layout() -> CoreLayout {
        CoreLayout::new(&[12, 4, 4])
    }

    fn load(smoothed: f32, sustained: f32) -> FocusLoad {
        FocusLoad {
            smoothed,
            recent: smoothed,
            sustained,
        }
    }

    #[test]
    fn the_footprint_widens_one_step_at_a_time_and_narrows_slowly() {
        let mut fp = Footprint::default();
        let mut at =
            |smoothed, sustained, now| fp.cores_for(KEY, load(smoothed, sustained), layout(), now);
        // 只靠一两个主线程：固定在大核。
        assert_eq!(at(150.0, 200.0, 0), CoreSet::Domains(0b1));
        // 超过大核能力（1200%）的 80%：多用一组小核（共 1600%）。
        assert_eq!(at(1000.0, 1000.0, 2_000), CoreSet::Domains(0b11));
        // 连续两轮都很高：再放开一级。
        assert_eq!(at(1000.0, 1900.0, 4_000), CoreSet::All);
        // 负载降下来也要先保持一会儿，再逐级收回。
        assert_eq!(at(300.0, 300.0, 8_000), CoreSet::All);
        assert_eq!(at(300.0, 300.0, 14_000), CoreSet::Domains(0b11));
        assert_eq!(at(300.0, 300.0, 16_000), CoreSet::Domains(0b1));
        // 在门槛之间：保持不变。
        assert_eq!(at(1000.0, 1000.0, 18_000), CoreSet::Domains(0b11));
        assert_eq!(at(700.0, 700.0, 40_000), CoreSet::Domains(0b11));
    }

    #[test]
    fn a_big_jump_widens_one_step_per_round() {
        let mut fp = Footprint::default();
        let mut at = |sustained, now| fp.cores_for(KEY, load(150.0, sustained), layout(), now);
        assert_eq!(at(150.0, 0), CoreSet::Domains(0b1));
        assert_eq!(at(1900.0, 2_000), CoreSet::Domains(0b11));
        assert_eq!(at(1900.0, 4_000), CoreSet::All);
    }

    #[test]
    fn the_tiers_follow_the_core_domains() {
        let one_group = CoreLayout::new(&[12, 8]);
        let mut fp = Footprint::default();
        assert_eq!(
            fp.cores_for(KEY, load(1000.0, 0.0), one_group, 0),
            CoreSet::All
        );
        // 双 CCD：先一个 CCD，不够再放开。
        let two_ccds = CoreLayout::with_cores(&[(16, 8), (16, 8)]);
        let mut fp = Footprint::default();
        assert_eq!(
            fp.cores_for(KEY, load(600.0, 600.0), two_ccds, 0),
            CoreSet::Domains(0b1)
        );
        // 一个 CCD 的 8 个核心 16 线程按 1000% 算：超线程挤在同一个物理核心上快不了多少。
        assert_eq!(
            fp.cores_for(KEY, load(850.0, 850.0), two_ccds, 2_000),
            CoreSet::All
        );
        let mut fp = Footprint::default();
        let uniform = CoreLayout::uniform(16);
        assert_eq!(fp.cores_for(KEY, load(50.0, 0.0), uniform, 0), CoreSet::All);
    }
}
