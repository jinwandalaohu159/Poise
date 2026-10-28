//! 安全守卫：所有动作执行前的唯一关卡。
//!
//! 规则集中在这里，各模块无法绕过：业务代码只拿得到 [`GuardedExecutor`]。

use std::fmt;

use crate::action::{Action, ActionExecutor, ActionKind, ExecError, ExecOutcome};
use crate::classify::{Classification, Role};
use crate::known::is_protected_service;
use crate::model::PriorityClass;
use crate::prefs::AppPreference;

/// 动作由谁触发。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 自动（调度、自动清理）。
    Auto,
    /// 用户点击按钮。
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// 关键、受保护、反作弊、虚拟机或 Poise 自己。
    Untouchable,
    /// 存在反作弊时，游戏只允许任务管理器等价的操作。
    AntiCheatGame,
    /// 只调度用户会话里的普通程序。
    NotUserApp,
    /// 正在被使用（前台、可见窗口、正在发声、用户要求保持快）。
    InUse,
    /// 最近几分钟在干活。
    RecentlyActive,
    /// 会造成短暂打扰的操作只能由用户手动触发。
    NeedsManual,
    /// 这个服务不能安全重启。
    ServiceNotRestartable,
    /// 这类进程不支持重启。
    NotRestartable,
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Untouchable => "受保护进程，永不触碰",
            Self::AntiCheatGame => "反作弊游戏只允许任务管理器等价的操作",
            Self::NotUserApp => "不是用户程序",
            Self::InUse => "正在使用中",
            Self::RecentlyActive => "最近在干活",
            Self::NeedsManual => "会短暂打扰，只能手动触发",
            Self::ServiceNotRestartable => "服务不能安全重启",
            Self::NotRestartable => "这类进程不支持重启",
        };
        f.write_str(s)
    }
}

pub struct SafetyGuard;

impl SafetyGuard {
    pub fn check(
        action: &Action,
        c: &Classification,
        original_priority: PriorityClass,
        trigger: Trigger,
    ) -> Result<(), DenyReason> {
        if c.is_untouchable() {
            return Err(DenyReason::Untouchable);
        }
        if c.is_game && c.anti_cheat_present && !action.is_task_manager_equivalent() {
            return Err(DenyReason::AntiCheatGame);
        }
        match action.kind(original_priority) {
            ActionKind::Restore => Ok(()),
            ActionKind::Boost => {
                if c.is_schedulable() {
                    Ok(())
                } else {
                    Err(DenyReason::NotUserApp)
                }
            }
            ActionKind::Demote => Self::check_demote(c),
            ActionKind::Restart => Self::check_restart(action, c, trigger),
        }
    }

    /// 与调度策略（`schedule::policy`）的规则对应、互为把关：改其中一边时两边一起改。
    fn check_demote(c: &Classification) -> Result<(), DenyReason> {
        if !c.is_schedulable() {
            return Err(DenyReason::NotUserApp);
        }
        // 正在发声的程序（整个进程树）、游戏完全不降级，连"低优先级"也让步。
        if c.plays_audio || c.is_game {
            return Err(DenyReason::InUse);
        }
        // 用户设为"低优先级"的程序在前台也留在小核。
        if c.preference == AppPreference::Background {
            return Ok(());
        }
        let in_use = c.in_foreground_tree
            || matches!(
                c.preference,
                AppPreference::AlwaysFast | AppPreference::Game
            );
        if in_use {
            return Err(DenyReason::InUse);
        }
        Ok(())
    }

    fn check_restart(
        action: &Action,
        c: &Classification,
        trigger: Trigger,
    ) -> Result<(), DenyReason> {
        if c.in_foreground_tree || c.has_visible_window || c.plays_audio {
            return Err(DenyReason::InUse);
        }
        // 窗口程序的一部分（例如终端里的命令行程序）：重启会破坏那个程序。
        if matches!(action, Action::RestartApp { .. }) && c.tree_has_window {
            return Err(DenyReason::InUse);
        }
        match action {
            Action::RecreateCompositor { .. } if c.role == Role::Compositor => {
                Self::require_manual(trigger)
            }
            Action::RestartService { service, .. } => {
                Self::check_service_restart(service, c, trigger)
            }
            Action::RestartApp { .. } if c.role == Role::UserApp => {
                if c.recently_active {
                    Err(DenyReason::RecentlyActive)
                } else {
                    Ok(())
                }
            }
            _ => Err(DenyReason::NotRestartable),
        }
    }

    fn check_service_restart(
        target: &str,
        c: &Classification,
        trigger: Trigger,
    ) -> Result<(), DenyReason> {
        // 目标服务必须正是分类得出的那个服务，防止借道重启别的服务。
        let matches_role = match &c.role {
            Role::ServiceHost => true,
            Role::ServiceChild { service } => service.eq_ignore_ascii_case(target),
            _ => false,
        };
        if !matches_role {
            return Err(DenyReason::NotRestartable);
        }
        Self::require_manual(trigger)?;
        let restartable = c
            .restartable_service
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case(target));
        if restartable && !is_protected_service(target) {
            Ok(())
        } else {
            Err(DenyReason::ServiceNotRestartable)
        }
    }

    fn require_manual(trigger: Trigger) -> Result<(), DenyReason> {
        if trigger == Trigger::Manual {
            Ok(())
        } else {
            Err(DenyReason::NeedsManual)
        }
    }
}

/// 唯一对业务开放的执行器：先过安全守卫，再交给平台层执行。
pub struct GuardedExecutor {
    inner: Box<dyn ActionExecutor>,
    dry_run: bool,
}

impl GuardedExecutor {
    /// `dry_run` 为真时只做安全检查、不真正执行，用于开发和观察模式。
    pub fn new(inner: Box<dyn ActionExecutor>, dry_run: bool) -> Self {
        Self { inner, dry_run }
    }

    pub fn execute(
        &self,
        action: &Action,
        c: &Classification,
        original_priority: PriorityClass,
        trigger: Trigger,
    ) -> Result<ExecOutcome, ExecError> {
        SafetyGuard::check(action, c, original_priority, trigger)
            .map_err(|r| ExecError::Denied(r.to_string()))?;
        if self.dry_run {
            return Ok(ExecOutcome::default());
        }
        self.inner.execute(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CoreSet, EcoQos};
    use crate::classify::ScheduleScope;

    fn user_app() -> Classification {
        Classification {
            role: Role::UserApp,
            root_pid: 1,
            in_foreground_tree: false,
            has_visible_window: false,
            tree_has_window: false,
            plays_audio: false,
            recently_active: false,
            preference: AppPreference::Auto,
            is_game: false,
            anti_cheat_present: false,
            restartable_service: None,
            schedule: ScheduleScope::App,
        }
    }

    fn check(action: Action, c: &Classification, trigger: Trigger) -> Result<(), DenyReason> {
        SafetyGuard::check(&action, c, PriorityClass::Normal, trigger)
    }

    const DEMOTE: Action = Action::SetEcoQos {
        pid: 1,
        state: EcoQos::On,
    };
    const BOOST: Action = Action::SetPriority {
        pid: 1,
        class: PriorityClass::High,
    };

    #[test]
    fn untouchable_roles_deny_everything_even_restore() {
        for role in [
            Role::Critical,
            Role::Protected,
            Role::AntiCheat,
            Role::VirtualMachine,
            Role::SelfProcess,
        ] {
            let c = Classification { role, ..user_app() };
            for a in [
                DEMOTE,
                BOOST,
                Action::SetCoreSet {
                    pid: 1,
                    set: CoreSet::All,
                },
                Action::RestartApp { pid: 1 },
            ] {
                assert_eq!(check(a, &c, Trigger::Manual), Err(DenyReason::Untouchable));
            }
        }
    }

    #[test]
    fn anti_cheat_game_allows_only_task_manager_actions() {
        let c = Classification {
            is_game: true,
            anti_cheat_present: true,
            ..user_app()
        };
        assert_eq!(check(BOOST, &c, Trigger::Auto), Ok(()));
        assert_eq!(
            check(
                Action::SetEcoQos {
                    pid: 1,
                    state: EcoQos::Off
                },
                &c,
                Trigger::Auto
            ),
            Ok(())
        );
        assert_eq!(
            check(Action::RestartApp { pid: 1 }, &c, Trigger::Manual),
            Err(DenyReason::AntiCheatGame)
        );
    }

    #[test]
    fn foreground_audio_and_always_fast_are_never_demoted() {
        for c in [
            Classification {
                in_foreground_tree: true,
                ..user_app()
            },
            Classification {
                plays_audio: true,
                ..user_app()
            },
            Classification {
                preference: AppPreference::AlwaysFast,
                ..user_app()
            },
            Classification {
                is_game: true,
                ..user_app()
            },
        ] {
            assert_eq!(check(DEMOTE, &c, Trigger::Auto), Err(DenyReason::InUse));
        }
        assert_eq!(check(DEMOTE, &user_app(), Trigger::Auto), Ok(()));
    }

    const TO_SMALL_CORES: Action = Action::SetCoreSet {
        pid: 1,
        set: CoreSet::Efficiency,
    };
    const LOWER: Action = Action::SetPriority {
        pid: 1,
        class: PriorityClass::BelowNormal,
    };

    #[test]
    fn audio_apps_are_not_demoted_at_all() {
        let c = Classification {
            plays_audio: true,
            ..user_app()
        };
        assert_eq!(
            check(TO_SMALL_CORES, &c, Trigger::Auto),
            Err(DenyReason::InUse)
        );
        assert_eq!(check(LOWER, &c, Trigger::Auto), Err(DenyReason::InUse));
    }

    #[test]
    fn low_priority_apps_stay_low_in_the_foreground() {
        let c = Classification {
            in_foreground_tree: true,
            preference: AppPreference::Background,
            ..user_app()
        };
        assert_eq!(check(TO_SMALL_CORES, &c, Trigger::Auto), Ok(()));
        assert_eq!(check(LOWER, &c, Trigger::Auto), Ok(()));
    }

    #[test]
    fn excluded_user_programs_are_not_scheduled() {
        let c = Classification {
            schedule: ScheduleScope::Excluded,
            ..user_app()
        };
        assert_eq!(
            check(DEMOTE, &c, Trigger::Auto),
            Err(DenyReason::NotUserApp)
        );
        assert_eq!(check(BOOST, &c, Trigger::Auto), Err(DenyReason::NotUserApp));
    }

    #[test]
    fn system_processes_are_not_scheduled() {
        let c = Classification {
            role: Role::SystemOther,
            schedule: ScheduleScope::Excluded,
            ..user_app()
        };
        assert_eq!(
            check(DEMOTE, &c, Trigger::Auto),
            Err(DenyReason::NotUserApp)
        );
        assert_eq!(check(BOOST, &c, Trigger::Auto), Err(DenyReason::NotUserApp));
    }

    #[test]
    fn windowed_or_active_apps_are_never_restarted() {
        let windowed = Classification {
            has_visible_window: true,
            ..user_app()
        };
        assert_eq!(
            check(Action::RestartApp { pid: 1 }, &windowed, Trigger::Manual),
            Err(DenyReason::InUse)
        );
        let part_of_windowed_app = Classification {
            tree_has_window: true,
            ..user_app()
        };
        assert_eq!(
            check(
                Action::RestartApp { pid: 1 },
                &part_of_windowed_app,
                Trigger::Manual
            ),
            Err(DenyReason::InUse)
        );
        let active = Classification {
            recently_active: true,
            ..user_app()
        };
        assert_eq!(
            check(Action::RestartApp { pid: 1 }, &active, Trigger::Manual),
            Err(DenyReason::RecentlyActive)
        );
        assert_eq!(
            check(Action::RestartApp { pid: 1 }, &user_app(), Trigger::Auto),
            Ok(())
        );
    }

    #[test]
    fn disruptive_restarts_need_manual_trigger() {
        let dwm = Classification {
            role: Role::Compositor,
            ..user_app()
        };
        let a = Action::RecreateCompositor { pid: 1 };
        assert_eq!(
            check(a.clone(), &dwm, Trigger::Auto),
            Err(DenyReason::NeedsManual)
        );
        assert_eq!(check(a, &dwm, Trigger::Manual), Ok(()));
    }

    #[test]
    fn service_restart_requires_restartable_and_unprotected_service() {
        let audio = Classification {
            role: Role::ServiceChild {
                service: "Audiosrv".into(),
            },
            restartable_service: Some("Audiosrv".into()),
            ..user_app()
        };
        let restart = |s: &str| Action::RestartService {
            pid: 1,
            service: s.into(),
        };
        assert_eq!(check(restart("Audiosrv"), &audio, Trigger::Manual), Ok(()));
        assert_eq!(
            check(restart("Audiosrv"), &audio, Trigger::Auto),
            Err(DenyReason::NeedsManual)
        );
        // 目标服务与分类不一致时拒绝，防止借道重启别的服务。
        assert_eq!(
            check(restart("Winmgmt"), &audio, Trigger::Manual),
            Err(DenyReason::NotRestartable)
        );
        let not_stoppable = Classification {
            restartable_service: None,
            ..audio.clone()
        };
        assert_eq!(
            check(restart("Audiosrv"), &not_stoppable, Trigger::Manual),
            Err(DenyReason::ServiceNotRestartable)
        );
        let host = Classification {
            role: Role::ServiceHost,
            restartable_service: Some("Winmgmt".into()),
            ..user_app()
        };
        assert_eq!(
            check(restart("Winmgmt"), &host, Trigger::Manual),
            Err(DenyReason::ServiceNotRestartable)
        );
    }

    struct Panicking;
    impl ActionExecutor for Panicking {
        fn execute(&self, _: &Action) -> Result<ExecOutcome, ExecError> {
            panic!("dry run must not execute");
        }
    }

    #[test]
    fn dry_run_checks_but_never_executes() {
        let ex = GuardedExecutor::new(Box::new(Panicking), true);
        assert!(
            ex.execute(&DEMOTE, &user_app(), PriorityClass::Normal, Trigger::Auto)
                .is_ok()
        );
        let denied = Classification {
            in_foreground_tree: true,
            ..user_app()
        };
        assert!(matches!(
            ex.execute(&DEMOTE, &denied, PriorityClass::Normal, Trigger::Auto),
            Err(ExecError::Denied(_))
        ));
    }
}
