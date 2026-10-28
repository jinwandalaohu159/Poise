//! 健康状态：电脑现在怎么样。

use serde::{Deserialize, Serialize};

use crate::cleanup::{Advisory, CleanupPlan};
use crate::model::SystemMetrics;
use crate::schedule::GameInfo;

use super::{DiagnoseConfig, Heat, is_storage_low, is_throttling};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCode {
    Smooth,
    InGame,
    Tired,
    Hot,
    RestartRecommended,
    /// 系统盘快满了（固态硬盘满了会变慢）。
    StorageLow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Good,
    Notice,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub code: HealthCode,
    pub level: Level,
    pub game: Option<String>,
    /// 一键清理能处理的项目数。
    pub cleanable: usize,
    pub freeable_bytes: u64,
}

/// 还没有评估结果时（刚启动）：按"运行顺畅"显示。
impl Default for Health {
    fn default() -> Self {
        Self {
            code: HealthCode::Smooth,
            level: Level::Good,
            game: None,
            cleanable: 0,
            freeable_bytes: 0,
        }
    }
}

pub struct HealthInput<'a> {
    pub system: &'a SystemMetrics,
    pub cpu_pct: f32,
    /// 由 [`super::HeatWatch`] 按平滑温度判断。
    pub heat: Heat,
    pub game: Option<&'a GameInfo>,
    /// 按"手动清理"预演出的计划，用来统计可清理项。
    pub manual_plan: &'a CleanupPlan,
    /// 系统盘（剩余，总大小）字节数。
    pub system_drive: Option<(u64, u64)>,
}

pub fn assess(cfg: &DiagnoseConfig, input: &HealthInput) -> Health {
    let throttling = is_throttling(cfg, input.system, input.cpu_pct);
    let plan = input.manual_plan;
    let cleanable = plan.run_now.len() + plan.deferred.len();
    let restart = plan
        .advisories
        .iter()
        .any(|a| matches!(a, Advisory::RestartRecommended { .. }));
    let tired = cleanable > 0 || input.system.commit_pct() >= cfg.commit_tired_pct;
    let storage_low = input
        .system_drive
        .is_some_and(|(free, total)| is_storage_low(cfg, free, total));

    let (code, level) = if input.heat == Heat::Critical {
        (HealthCode::Hot, Level::Bad)
    } else if input.heat == Heat::Hot || throttling {
        (HealthCode::Hot, Level::Notice)
    } else if input.game.is_some() {
        (HealthCode::InGame, Level::Good)
    } else if restart {
        (HealthCode::RestartRecommended, Level::Notice)
    } else if storage_low {
        (HealthCode::StorageLow, Level::Notice)
    } else if tired {
        (HealthCode::Tired, Level::Notice)
    } else {
        (HealthCode::Smooth, Level::Good)
    };
    Health {
        code,
        level,
        game: input.game.map(|g| g.name.clone()),
        cleanable,
        freeable_bytes: plan.total_freeable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::cleanup::Advisory;
    use crate::cleanup::{CleanupReason, CleanupStep};
    use crate::model::ProcessKey;

    fn step() -> CleanupStep {
        CleanupStep {
            key: ProcessKey::default(),
            action: Action::RestartApp { pid: 1 },
            disruptive: false,
            reason: CleanupReason::Oversized,
            name: "x".into(),
            memory_bytes: 100,
        }
    }

    fn input<'a>(
        system: &'a SystemMetrics,
        plan: &'a CleanupPlan,
        heat: Heat,
        game: Option<&'a GameInfo>,
    ) -> HealthInput<'a> {
        HealthInput {
            system,
            cpu_pct: 10.0,
            heat,
            game,
            manual_plan: plan,
            system_drive: None,
        }
    }

    #[test]
    fn smooth_when_nothing_to_do() {
        let sys = SystemMetrics::default();
        let plan = CleanupPlan::default();
        let h = assess(
            &DiagnoseConfig::default(),
            &input(&sys, &plan, Heat::Normal, None),
        );
        assert_eq!(h.code, HealthCode::Smooth);
    }

    #[test]
    fn heat_outranks_everything() {
        let sys = SystemMetrics::default();
        let plan = CleanupPlan {
            run_now: vec![step()],
            ..Default::default()
        };
        let game = GameInfo {
            pid: 1,
            name: "LOL".into(),
        };
        let h = assess(
            &DiagnoseConfig::default(),
            &input(&sys, &plan, Heat::Critical, Some(&game)),
        );
        assert_eq!((h.code, h.level), (HealthCode::Hot, Level::Bad));
    }

    #[test]
    fn game_is_shown_and_cleanable_items_make_it_tired() {
        let sys = SystemMetrics::default();
        let plan = CleanupPlan {
            run_now: vec![step()],
            ..Default::default()
        };
        let game = GameInfo {
            pid: 1,
            name: "LOL".into(),
        };
        let cfg = DiagnoseConfig::default();
        let h = assess(&cfg, &input(&sys, &plan, Heat::Normal, Some(&game)));
        assert_eq!(h.code, HealthCode::InGame);
        assert_eq!(h.game.as_deref(), Some("LOL"));
        let h = assess(&cfg, &input(&sys, &plan, Heat::Normal, None));
        assert_eq!(h.code, HealthCode::Tired);
        assert_eq!(h.cleanable, 1);
    }

    #[test]
    fn restart_recommendation_is_bad() {
        let sys = SystemMetrics::default();
        let plan = CleanupPlan {
            advisories: vec![Advisory::RestartRecommended { kernel_nonpaged: 1 }],
            ..Default::default()
        };
        let h = assess(
            &DiagnoseConfig::default(),
            &input(&sys, &plan, Heat::Normal, None),
        );
        assert_eq!(h.code, HealthCode::RestartRecommended);
    }

    #[test]
    fn nearly_full_system_drive_is_noticed() {
        const GB: u64 = 1024 * 1024 * 1024;
        let sys = SystemMetrics::default();
        let plan = CleanupPlan::default();
        let cfg = DiagnoseConfig::default();
        let mut i = input(&sys, &plan, Heat::Normal, None);
        i.system_drive = Some((600 * GB, 780 * GB));
        assert_eq!(assess(&cfg, &i).code, HealthCode::Smooth);
        i.system_drive = Some((60 * GB, 780 * GB));
        assert_eq!(assess(&cfg, &i).code, HealthCode::StorageLow);
        // 小硬盘：比例够但绝对值太少。
        i.system_drive = Some((12 * GB, 100 * GB));
        assert_eq!(assess(&cfg, &i).code, HealthCode::StorageLow);
    }

    #[test]
    fn throttling_detected_under_load_below_base_clock() {
        let cfg = DiagnoseConfig::default();
        let sys = SystemMetrics {
            cpu_mhz_current: 1500,
            cpu_mhz_max: 2300,
            ..Default::default()
        };
        assert!(is_throttling(&cfg, &sys, 90.0));
        assert!(!is_throttling(&cfg, &sys, 20.0));
    }
}
