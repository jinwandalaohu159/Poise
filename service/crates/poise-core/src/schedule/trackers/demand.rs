//! 资源需求：此刻有没有正在用的程序需要处理器。
//!
//! 让路是为了把资源让给正在用的程序；没人需要的时候让路只会白白拖慢后台的工作
//! （例如用户离开电脑时的下载、编译）。所以先判断需求，再决定后台要不要让路、让到什么程度：
//!
//! | 需求 | 情况 | 后台怎样算在忙 |
//! |---|---|---|
//! | 高 | 正在玩游戏；处理器接近温度上限（功耗和温度要留给正在用的程序） | 很低的负载就让路 |
//! | 一般 | 前台程序在忙；整机繁忙；用电池（让路的同时省电） | 一般 |
//! | 无 | 以上都不是 | 不让路；已经让路的在持续无需求一段时间后恢复 |
//!
//! 需求出现后保持一段时间（前台的负载时有时无，例如滚动网页），不会每轮来回变化。

use crate::model::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Pressure {
    #[default]
    None,
    Normal,
    High,
}

/// 判断需求的依据（每轮一份）。
#[derive(Debug, Clone, Copy, Default)]
pub struct DemandSignals {
    pub gaming: bool,
    /// 处理器接近温度上限。
    pub hot: bool,
    pub on_battery: bool,
    /// 前台进程树此刻的负载（单核百分比）。
    pub foreground_core_pct: f32,
    /// 整机处理器占用（0～100）。
    pub system_pct: f32,
}

/// 需求的判断规则。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemandRule {
    /// 前台进程树的负载达到这么多（单核百分比）算前台在忙。
    pub foreground_core_pct: f32,
    /// 整机处理器占用达到这么多算整机繁忙。
    pub system_pct: f32,
    /// 需求出现后保持多久。
    pub hold_ms: Millis,
}

#[derive(Debug, Default)]
pub struct DemandTracker {
    last_high_at: Option<Millis>,
    last_normal_at: Option<Millis>,
    /// 从什么时候开始没有需求。
    none_since: Option<Millis>,
}

impl DemandTracker {
    /// 每轮调用一次，返回此刻的需求。
    pub fn update(&mut self, s: DemandSignals, now: Millis, rule: DemandRule) -> Pressure {
        if s.gaming || s.hot {
            self.last_high_at = Some(now);
        }
        if s.on_battery
            || s.foreground_core_pct >= rule.foreground_core_pct
            || s.system_pct >= rule.system_pct
        {
            self.last_normal_at = Some(now);
        }
        let recent = |t: Option<Millis>| t.is_some_and(|t| now.saturating_sub(t) < rule.hold_ms);
        let pressure = if recent(self.last_high_at) {
            Pressure::High
        } else if recent(self.last_normal_at) {
            Pressure::Normal
        } else {
            Pressure::None
        };
        if pressure == Pressure::None {
            self.none_since.get_or_insert(now);
        } else {
            self.none_since = None;
        }
        pressure
    }

    /// 已经连续多久没有需求（有需求时为 0）。
    pub fn idle_for(&self, now: Millis) -> Millis {
        self.none_since.map_or(0, |t| now.saturating_sub(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: DemandRule = DemandRule {
        foreground_core_pct: 25.0,
        system_pct: 50.0,
        hold_ms: 30_000,
    };

    #[test]
    fn demand_is_held_for_a_while_and_then_released() {
        let mut d = DemandTracker::default();
        let busy_foreground = DemandSignals {
            foreground_core_pct: 60.0,
            ..Default::default()
        };
        assert_eq!(d.update(busy_foreground, 0, RULE), Pressure::Normal);
        // 前台闲下来：保持一会儿。
        assert_eq!(
            d.update(DemandSignals::default(), 20_000, RULE),
            Pressure::Normal
        );
        assert_eq!(
            d.update(DemandSignals::default(), 40_000, RULE),
            Pressure::None
        );
        assert_eq!(d.idle_for(70_000), 30_000);
        let gaming = DemandSignals {
            gaming: true,
            ..Default::default()
        };
        assert_eq!(d.update(gaming, 80_000, RULE), Pressure::High);
        assert_eq!(d.idle_for(80_000), 0);
    }

    #[test]
    fn battery_and_heat_create_demand_on_their_own() {
        let mut d = DemandTracker::default();
        let battery = DemandSignals {
            on_battery: true,
            ..Default::default()
        };
        assert_eq!(d.update(battery, 0, RULE), Pressure::Normal);
        let hot = DemandSignals {
            hot: true,
            ..Default::default()
        };
        assert_eq!(d.update(hot, 2_000, RULE), Pressure::High);
    }
}
