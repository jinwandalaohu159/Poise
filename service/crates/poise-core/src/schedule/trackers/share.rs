//! 有重点程序（游戏、前台的重活）时，后台的工作和它怎样分处理器：主域（大核）按物理核心切开，
//! 系统优先使用的那几个（睿频最高的"优选核心"）留给重点程序，后台的工作用剩下的大核和全部小核
//! （见 `CoreSet::Rest`）。两边不共用物理核心：超线程的两个线程挤在同一个物理核心上时，优先级帮不上忙。
//!
//! 留几个按重点程序此刻实际用了多少决定：留出的核心只用到一部分（见 [`ShareRule::target_util`]），
//! 再多留 [`ShareRule::spare_cores`] 个应付突发；它用得多了立即多留，用得少了持续一阵才收回，后台的工作
//! 不跟着负载的起伏来回搬。重点程序需要整个主域时，后台的工作只用小核。
//!
//! 再看结果：留出的那几个核心本身已经很忙（[`ShareRule::crowded_pct`]），说明重点程序在里面挤着——
//! 不论按负载算出来要几个，都再多留一个，直到不挤为止。挤过之后记住"这么重的时候这几个不够"：负载明显
//! 降下来（见 [`RELIEF_RATIO`]）之前不再收回到挤的那个数——否则一分钟后按负载收回、又挤、又多留，来回改。

use crate::model::Millis;

/// 挤过之后，负载降到挤的时候的这个比例以下才可以收回到挤的那个数以下。
const RELIEF_RATIO: f32 = 0.75;

/// 怎样决定留几个物理核心。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShareRule {
    /// 留出的核心按每个 100%（一个线程跑满）算，重点程序的负载只占到这个比例：游戏的负载是几秒内的平均，
    /// 一帧之内的突发高得多。
    pub target_util: f32,
    /// 另外多留几个。
    pub spare_cores: u32,
    /// 至少留几个。
    pub min_cores: u32,
    /// 需要的变少后，持续这么久才收回。
    pub narrow_after_ms: Millis,
    /// 留出的核心平均忙到这个程度（0～100，一个线程跑满算 100）就再多留一个。
    pub crowded_pct: f32,
}

#[derive(Debug, Default)]
pub struct CoreShare {
    kept: u32,
    /// 需要的比留着的少，从什么时候开始。
    fewer_since: Option<Millis>,
    /// 挤过：至少留几个，以及挤的时候的负载（负载明显降下来之前不收回到这以下）。
    crowded_floor: Option<(u32, f32)>,
}

impl CoreShare {
    /// 每轮有重点程序时调用：`load` 是重点程序合计的负载（单核百分比），`main_cores` 是主域的物理核心数，
    /// `core_busy` 是主域各物理核心此刻有多忙（按系统优先使用的顺序；读不到时为空）。
    /// 返回留给重点程序的物理核心数（不超过 `main_cores`；等于时整个主域都留给它）。
    pub fn update(
        &mut self,
        load: f32,
        main_cores: u32,
        core_busy: &[f32],
        rule: ShareRule,
        now: Millis,
    ) -> u32 {
        let per_core = 100.0 * rule.target_util;
        let mut need = (load.max(0.0) / per_core).ceil() as u32 + rule.spare_cores;
        if self.crowded(core_busy, rule) {
            need = need.max(self.kept + 1);
            let floor = self.crowded_floor.map_or(need, |(f, _)| f.max(need));
            self.crowded_floor = Some((floor, load));
        } else if self
            .crowded_floor
            .is_some_and(|(_, at)| load < at * RELIEF_RATIO)
        {
            self.crowded_floor = None;
        }
        if let Some((floor, _)) = self.crowded_floor {
            need = need.max(floor);
        }
        let need = need.max(rule.min_cores).min(main_cores);
        if need >= self.kept {
            self.kept = need;
            self.fewer_since = None;
        } else {
            let since = *self.fewer_since.get_or_insert(now);
            if now.saturating_sub(since) >= rule.narrow_after_ms {
                self.kept = need;
                self.fewer_since = None;
            }
        }
        self.kept.min(main_cores)
    }

    /// 留出的核心平均忙到了 [`ShareRule::crowded_pct`]。
    fn crowded(&self, core_busy: &[f32], rule: ShareRule) -> bool {
        let kept = (self.kept as usize).min(core_busy.len());
        kept > 0 && core_busy[..kept].iter().sum::<f32>() / kept as f32 >= rule.crowded_pct
    }

    /// 没有重点程序了：下次从头算。
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: ShareRule = ShareRule {
        target_util: 0.7,
        spare_cores: 1,
        min_cores: 2,
        narrow_after_ms: 60_000,
        crowded_pct: 80.0,
    };

    #[test]
    fn crowded_kept_cores_get_one_more_whatever_the_load_says() {
        let mut s = CoreShare::default();
        assert_eq!(s.update(150.0, 6, &[], RULE, 0), 4);
        // 按负载 4 个够了，可这 4 个都快跑满：多留一个，还挤就再多留。
        let busy = [90.0, 85.0, 95.0, 80.0, 10.0, 5.0];
        assert_eq!(s.update(150.0, 6, &busy, RULE, 2_000), 5);
        assert_eq!(
            s.update(150.0, 6, &[90.0, 90.0, 90.0, 90.0, 90.0, 5.0], RULE, 4_000),
            6
        );
        // 不挤了，可负载没变：挤的那个数不够，不收回（收回去又会挤）。
        let calm = [40.0; 6];
        assert_eq!(s.update(150.0, 6, &calm, RULE, 6_000), 6);
        assert_eq!(s.update(150.0, 6, &calm, RULE, 66_000), 6);
        assert_eq!(s.update(150.0, 6, &calm, RULE, 300_000), 6);
        // 负载明显降下来：持续一分钟后按负载收回。
        assert_eq!(s.update(100.0, 6, &calm, RULE, 302_000), 6);
        assert_eq!(s.update(100.0, 6, &calm, RULE, 362_000), 3);
    }

    #[test]
    fn the_focus_keeps_cores_by_what_it_uses_and_gets_more_at_once() {
        let mut s = CoreShare::default();
        // 英雄联盟这样只靠两三个线程的游戏：留 4 个，后台的工作用剩下 2 个大核和全部小核。
        assert_eq!(s.update(150.0, 6, &[], RULE, 0), 4);
        // 团战时用得多：立即多留。
        assert_eq!(s.update(350.0, 6, &[], RULE, 2_000), 6);
        // 回到平时：持续一分钟才收回。
        assert_eq!(s.update(150.0, 6, &[], RULE, 4_000), 6);
        assert_eq!(s.update(150.0, 6, &[], RULE, 64_000), 4);
        // 闲着（菜单里）也至少留 2 个。
        assert_eq!(s.update(0.0, 6, &[], RULE, 200_000), 4);
        assert_eq!(s.update(0.0, 6, &[], RULE, 300_000), 2);
        // 主域核心少：不超过主域。
        s.reset();
        assert_eq!(s.update(400.0, 4, &[], RULE, 0), 4);
    }
}
