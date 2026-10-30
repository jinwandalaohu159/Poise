//! 守住 Poise 写入的电源设置。电源计划是公共的：厂商工具、加速器、系统更新都可能改它，
//! 实测有电脑开机一分钟内最小处理器状态就被改回 100%。这里：
//!
//! - 电源计划被切换：当前模式重新写入新计划（旧计划里写过的值由变更日志写回旧计划）；
//! - Poise 写入的值被别处改掉：改回来，并记下被改成了什么——日志里的时间可以对照是谁改的；
//! - 用户应用过的电源优化项不再是优化状态（例如新版本覆盖了更多处理器等级）：重新应用，补上新的位置；
//! - Windows 电源模式滑块盖住了计划里的设置：记一条日志说明（滑块是用户的选择，不替用户改）。
//!
//! 只读几个电源设置，每次不到一毫秒。

use poise_core::journal::JournalEntry;
use poise_core::model::Millis;
use poise_core::optimize::{self, OptimizationItem};
use poise_core::power_settings;
use poise_core::setting::{Target, Value};
use poise_platform::settings::power_plan;
use poise_protocol::Event;

use super::Engine;
use super::changes::GAME_PREFIX;

/// 平时多久检查一次有没有被改掉。
const CHECK_EVERY_MS: Millis = 60_000;
/// 服务启动、用户登录、换了电源计划之后的一段时间里查得勤一些：厂商工具多半在这时改电源设置
/// （实测开机后两秒就被改掉了）。
const WATCHFUL_EVERY_MS: Millis = 10_000;
const WATCHFUL_FOR_MS: Millis = 3 * 60_000;

#[derive(Default)]
pub(super) struct PowerGuard {
    checked_at: Option<Millis>,
    /// 勤查到什么时候；None 表示从服务启动算起（引擎的时间从 0 开始）。
    watchful_until: Option<Millis>,
    /// 上次看到的当前电源计划。
    scheme: Option<String>,
    /// 上次看到的电源模式滑块（None 表示没有覆盖）。
    overlay: Option<Option<String>>,
}

impl PowerGuard {
    /// 下次采样时立即检查，之后勤查一段时间（例如刚登录，用户的程序正在启动）。
    pub fn check_soon(&mut self, now: Millis) {
        self.checked_at = None;
        self.watchful_until = Some(now + WATCHFUL_FOR_MS);
    }

    fn due(&self, now: Millis) -> bool {
        let every = if now < self.watchful_until.unwrap_or(WATCHFUL_FOR_MS) {
            WATCHFUL_EVERY_MS
        } else {
            CHECK_EVERY_MS
        };
        self.checked_at
            .is_none_or(|t| now.saturating_sub(t) >= every)
    }
}

impl Engine {
    /// 每轮采样调用。观察模式下不写入，什么都不做。
    pub(super) fn guard_power(&mut self, now: Millis) {
        if self.cfg.dry_run {
            return;
        }
        let scheme = active_scheme();
        if scheme != self.power_guard.scheme {
            let previous = std::mem::replace(&mut self.power_guard.scheme, scheme.clone());
            if let Some(old) = previous {
                self.logbook.event_with(
                    Event::PlanSwitched,
                    format!("{old} → {}", scheme.as_deref().unwrap_or("?")),
                );
                self.hardware.power_plan_changed();
                self.power_guard.check_soon(now);
            }
        }
        if !self.power_guard.due(now) {
            return;
        }
        self.power_guard.checked_at = Some(now);
        self.note_overlay();
        if let Some(scheme) = &scheme {
            self.restore_drifted(scheme, now);
        }
        self.complete_power_items(now);
    }

    /// Poise 在当前计划里写入、又被别处改掉的电源设置：改回来。
    fn restore_drifted(&mut self, scheme: &str, now: Millis) {
        let drifted = self.changes.journal().drifted(&self.changes.store, |e| {
            !e.owner.starts_with(GAME_PREFIX) && in_scheme(&e.target, scheme)
        });
        if drifted.is_empty() {
            return;
        }
        for (e, current) in &drifted {
            let Target::PowerSetting { setting, ac, .. } = &e.target else {
                continue;
            };
            let kind = power_settings::kind(setting);
            self.logbook.event_with(
                Event::SettingRestored {
                    setting: kind.map(|(k, _)| k),
                    class: kind.and_then(|(_, c)| c),
                    ac: *ac,
                    found: dword(current.as_ref()),
                    expected: dword(Some(&e.applied)),
                },
                format!("{setting}，由 {} 设置", e.owner),
            );
        }
        let entries: Vec<JournalEntry> = drifted.into_iter().map(|(e, _)| e).collect();
        if let Err(e) = self.changes.reassert(&entries, now) {
            log::warn!("改回电源设置失败：{e}");
        }
    }

    /// 用户应用过、现在却不是优化状态的电源项：重新应用。
    fn complete_power_items(&mut self, now: Millis) {
        let items = optimize::power_items();
        let reverted: Vec<OptimizationItem> =
            optimize::reverted_power_items(&items, &self.changes.store, self.changes.journal())
                .into_iter()
                .cloned()
                .collect();
        for item in reverted {
            match self.changes.edit(|s, j| optimize::apply(&item, s, j, now)) {
                Ok(()) => self.logbook.event(Event::OptimizationReapplied {
                    id: item.id.clone(),
                }),
                Err(e) => log::warn!("优化项 {} 已不是优化状态，重新应用失败：{e}", item.id),
            }
        }
    }

    /// 电源模式滑块变化时记一条日志：处于"最佳能效"时，它会盖住计划里的能效偏好、最小处理器状态等。
    fn note_overlay(&mut self) {
        let overlay = power_plan::effective_overlay();
        let text = overlay.map(|g| power_plan::guid_text(&g));
        if self.power_guard.overlay.as_ref() == Some(&text) {
            return;
        }
        if overlay == Some(power_plan::OVERLAY_BETTER_BATTERY) {
            self.logbook.event(Event::EfficiencySlider);
        } else if self.power_guard.overlay.is_some() {
            log::info!(
                "Windows 电源模式滑块：{}",
                text.as_deref().unwrap_or("平衡（无覆盖）")
            );
        }
        self.power_guard.overlay = Some(text);
    }
}

fn active_scheme() -> Option<String> {
    power_plan::active_scheme()
        .ok()
        .map(|g| power_plan::guid_text(&g))
}

fn in_scheme(target: &Target, active: &str) -> bool {
    matches!(target, Target::PowerSetting { scheme: Some(s), .. } if s.eq_ignore_ascii_case(active))
}

/// 电源设置的数值；不存在或不是数值时为空。
fn dword(v: Option<&Value>) -> Option<u32> {
    match v {
        Some(Value::Dword(n)) => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_often_after_start_and_login_then_every_minute() {
        let mut g = PowerGuard::default();
        assert!(g.due(0));
        g.checked_at = Some(0);
        assert!(!g.due(5_000));
        assert!(g.due(WATCHFUL_EVERY_MS));
        g.checked_at = Some(WATCHFUL_FOR_MS);
        assert!(!g.due(WATCHFUL_FOR_MS + WATCHFUL_EVERY_MS));
        assert!(g.due(WATCHFUL_FOR_MS + CHECK_EVERY_MS));
        // 登录：立即查，之后又勤查一段时间。
        let login = 10 * 60_000;
        g.check_soon(login);
        assert!(g.due(login));
        g.checked_at = Some(login);
        assert!(g.due(login + WATCHFUL_EVERY_MS));
    }

    #[test]
    fn only_settings_in_the_active_scheme_are_guarded() {
        let t = |scheme: Option<&str>| Target::PowerSetting {
            scheme: scheme.map(Into::into),
            subgroup: power_settings::SUB_PROCESSOR.into(),
            setting: power_settings::BOOST_MODE.into(),
            ac: true,
        };
        assert!(in_scheme(&t(Some("ABC")), "abc"));
        assert!(!in_scheme(&t(Some("def")), "abc"));
        assert!(!in_scheme(&t(None), "abc"));
        assert_eq!(dword(Some(&Value::Dword(2))), Some(2));
        assert_eq!(dword(None), None);
    }
}
