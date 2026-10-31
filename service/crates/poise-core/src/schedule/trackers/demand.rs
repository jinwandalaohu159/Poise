//! 资源需求：此刻有没有正在用的程序需要处理器。
//!
//! 让路是为了把资源让给正在用的程序；没人需要的时候让路只会白白拖慢后台的工作
//! （例如用户离开电脑时的下载、编译）。所以先判断需求，再决定后台要不要让路、让到什么程度：
//!
//! | 需求 | 情况 | 后台怎样算在忙 |
//! |---|---|---|
//! | 高 | 有重点程序（游戏会话中、正在用的程序在持续干重活，结束后保持一会儿）；处理器接近温度上限 | 很低的负载就让路 |
//! | 一般 | 正在用的程序在忙；整机繁忙、并且正在用的程序也在动；用电池（让路的同时省电） | 一般 |
//! | 无 | 以上都不是 | 不让路；已经让路的在持续无需求一段时间后恢复 |
//!
//! 需求只看正在用的程序：后台程序自己把整机跑满，不算有人需要——否则它会为"没有人"让路，
//! 白白慢下来。需求出现后保持一段时间（前台的负载时有时无，例如滚动网页），不会每轮来回变化。

use serde::{Deserialize, Serialize};

use crate::model::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pressure {
    #[default]
    None,
    Normal,
    High,
}

/// 需求从哪来（日志用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemandReason {
    /// 有重点程序（正在玩的游戏，或正在用、持续干重活的程序）。
    Focus,
    /// 处理器没有余量（被限速，或读不到限速时温度贴着上限）。
    Heat,
    /// 正在用的程序在忙。
    InUse,
    /// 用电池：让路的同时省电。
    Battery,
}

/// 判断需求的依据（每轮一份）。
#[derive(Debug, Clone, Copy, Default)]
pub struct DemandSignals {
    /// 有重点程序（正在玩的游戏，或正在用、持续干重活的程序）。
    pub focused: bool,
    /// 处理器接近温度上限。
    pub hot: bool,
    pub on_battery: bool,
    /// 正在用的程序此刻的负载之和（单核百分比）。
    pub in_use_core_pct: f32,
    /// 整机处理器占用（0～100）。
    pub system_pct: f32,
}

/// 需求的判断规则。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemandRule {
    /// 正在用的程序的负载达到这么多（单核百分比）算它们在忙。
    pub in_use_core_pct: f32,
    /// 整机处理器占用达到这么多算整机繁忙。
    pub system_pct: f32,
    /// 整机繁忙时，正在用的程序的负载达到这么多（单核百分比）就算需要处理器：整机被占满时，
    /// 正在用的程序拿不到处理器，量到的负载本来就偏低。
    pub in_use_active_pct: f32,
    /// 需求出现后保持多久。
    pub hold_ms: Millis,
}

#[derive(Debug, Default)]
pub struct DemandTracker {
    last_high_at: Option<Millis>,
    last_normal_at: Option<Millis>,
    /// 最近一次需求高、一般的原因。
    high_reason: Option<DemandReason>,
    normal_reason: Option<DemandReason>,
    /// 从什么时候开始没有需求。
    none_since: Option<Millis>,
}

impl DemandTracker {
    /// 每轮调用一次，返回此刻的需求。
    pub fn update(&mut self, s: DemandSignals, now: Millis, rule: DemandRule) -> Pressure {
        if s.focused || s.hot {
            self.last_high_at = Some(now);
            self.high_reason = Some(if s.focused {
                DemandReason::Focus
            } else {
                DemandReason::Heat
            });
        }
        let in_use = s.in_use_core_pct >= rule.in_use_core_pct
            || (s.system_pct >= rule.system_pct && s.in_use_core_pct >= rule.in_use_active_pct);
        if s.on_battery || in_use {
            self.last_normal_at = Some(now);
            self.normal_reason = Some(if in_use {
                DemandReason::InUse
            } else {
                DemandReason::Battery
            });
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

    /// 这一级需求从哪来（最近一次的原因；没有需求时为空）。
    pub fn reason(&self, pressure: Pressure) -> Option<DemandReason> {
        match pressure {
            Pressure::None => None,
            Pressure::Normal => self.normal_reason,
            Pressure::High => self.high_reason,
        }
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
        in_use_core_pct: 25.0,
        system_pct: 50.0,
        in_use_active_pct: 5.0,
        hold_ms: 30_000,
    };

    #[test]
    fn demand_is_held_for_a_while_and_then_released() {
        let mut d = DemandTracker::default();
        let busy_foreground = DemandSignals {
            in_use_core_pct: 60.0,
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
            focused: true,
            ..Default::default()
        };
        assert_eq!(d.update(gaming, 80_000, RULE), Pressure::High);
        assert_eq!(d.idle_for(80_000), 0);
    }

    #[test]
    fn a_busy_machine_counts_only_when_someone_in_use_is_active() {
        let mut d = DemandTracker::default();
        // 后台自己把整机跑满，用户没在动：没人需要。
        let alone = DemandSignals {
            system_pct: 95.0,
            in_use_core_pct: 1.0,
            ..Default::default()
        };
        assert_eq!(d.update(alone, 0, RULE), Pressure::None);
        // 用户在动（量到的负载被挤得偏低）：算有需要。
        let squeezed = DemandSignals {
            in_use_core_pct: 8.0,
            ..alone
        };
        assert_eq!(d.update(squeezed, 2_000, RULE), Pressure::Normal);
    }

    #[test]
    fn battery_and_heat_create_demand_on_their_own() {
        let mut d = DemandTracker::default();
        let battery = DemandSignals {
            on_battery: true,
            ..Default::default()
        };
        assert_eq!(d.update(battery, 0, RULE), Pressure::Normal);
        assert_eq!(d.reason(Pressure::Normal), Some(DemandReason::Battery));
        let hot = DemandSignals {
            hot: true,
            ..Default::default()
        };
        assert_eq!(d.update(hot, 2_000, RULE), Pressure::High);
        assert_eq!(d.reason(Pressure::High), Some(DemandReason::Heat));
        assert_eq!(d.reason(Pressure::None), None);
    }
}
