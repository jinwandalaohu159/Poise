//! 守住 Poise 写入的电源设置。电源计划是公共的：厂商工具、加速器、系统更新都可能改它，
//! 实测有电脑开机一分钟内最小处理器状态就被改回 100%。这里：
//!
//! - 电源计划被切换：当前模式重新写入新计划（旧计划里写过的值由变更日志写回旧计划）；
//! - Poise 写入的值被别处改掉：改回来，并记下被改成了什么——日志里的时间可以对照是谁改的；
//! - 用户应用过的电源优化项不再是优化状态（例如新版本覆盖了更多处理器等级）：重新应用，补上新的位置；
//! - Windows 电源模式滑块盖住了计划里的设置：记一条日志说明（滑块是用户的选择，不替用户改）。
//!
//! 读都在电源设置读取线程里（[`check`]，见 [`crate::power_reader`]）：每 10 秒看一眼当前电源计划，到时间时
//! 做一次完整检查；引擎下一轮取结果，只在真有变化时写入（这里）。电源服务忙的时候读一次要几百毫秒，
//! 调度从不等它。还排在电源执行线程里的写入，检查时当作已经写下：刚切换完模式也不会误认成被改掉。

mod check;

use poise_core::journal::JournalEntry;
use poise_core::model::Millis;
use poise_core::optimize::{self, OptimizationItem};
use poise_core::power_settings;
use poise_core::setting::{Target, Value};
use poise_platform::settings::power_plan;
use poise_protocol::Event;
use windows::core::GUID;

use super::Engine;
use crate::power_reader::PowerReader;
use check::{PowerCheck, PowerRead, guarded};

/// 平时多久检查一次有没有被改掉。
const CHECK_EVERY_MS: Millis = 60_000;
/// 服务启动、用户登录、换了电源计划之后的一段时间里查得勤一些：厂商工具多半在这时改电源设置
/// （实测开机后两秒就被改掉了）。
const WATCHFUL_EVERY_MS: Millis = 10_000;
const WATCHFUL_FOR_MS: Millis = 3 * 60_000;
/// 两次完整检查之间，多久看一眼当前电源计划（每次都要问电源服务；换了计划晚几秒发现没关系）。
const SCHEME_EVERY_MS: Millis = 10_000;
/// Poise 暂停着（勤查期过后）时放慢：只守着用户选的优化项，被改掉晚几分钟改回无妨，不为它一夜问几千次电源服务。
const PAUSED_CHECK_EVERY_MS: Millis = 5 * 60_000;
const PAUSED_SCHEME_EVERY_MS: Millis = 60_000;

pub(super) struct PowerGuard {
    reader: PowerReader<PowerRead>,
    cadence: Cadence,
    /// 上次看到的当前电源计划。
    scheme: Option<String>,
    /// 上次看到的电源模式滑块（None 表示还没看过）。
    overlay: Option<Option<String>>,
}

impl PowerGuard {
    pub fn start() -> Self {
        Self {
            reader: PowerReader::start(),
            cadence: Cadence::default(),
            scheme: None,
            overlay: None,
        }
    }

    /// 下次采样时立即检查，之后勤查一段时间（例如刚登录，用户的程序正在启动）。
    pub fn check_soon(&mut self, now: Millis) {
        self.cadence.check_soon(now);
    }
}

/// 什么时候做完整检查、看一眼当前电源计划。
#[derive(Default)]
struct Cadence {
    checked_at: Option<Millis>,
    scheme_at: Option<Millis>,
    /// 勤查到什么时候；None 表示从服务启动算起（引擎的时间从 0 开始）。
    watchful_until: Option<Millis>,
}

impl Cadence {
    fn check_soon(&mut self, now: Millis) {
        self.checked_at = None;
        self.watchful_until = Some(now + WATCHFUL_FOR_MS);
    }

    fn watchful(&self, now: Millis) -> bool {
        now < self.watchful_until.unwrap_or(WATCHFUL_FOR_MS)
    }

    /// `paused`：Poise 暂停着（勤查期内照样勤查：开机、登录时厂商工具正在改设置）。
    fn due(&self, now: Millis, paused: bool) -> bool {
        let every = if self.watchful(now) {
            WATCHFUL_EVERY_MS
        } else if paused {
            PAUSED_CHECK_EVERY_MS
        } else {
            CHECK_EVERY_MS
        };
        self.checked_at
            .is_none_or(|t| now.saturating_sub(t) >= every)
    }

    fn scheme_due(&self, now: Millis, paused: bool) -> bool {
        let every = if paused && !self.watchful(now) {
            PAUSED_SCHEME_EVERY_MS
        } else {
            SCHEME_EVERY_MS
        };
        self.scheme_at
            .is_none_or(|t| now.saturating_sub(t) >= every)
    }
}

impl Engine {
    /// 每轮采样调用：处理读取线程交回的结果，到时间时再提交下一次读取（上一次还没读完时不提交）。
    /// 观察模式下不写入，什么都不做。
    pub(super) fn guard_power(&mut self, now: Millis) {
        if self.cfg.dry_run {
            return;
        }
        for read in self.power_guard.reader.results() {
            match read {
                PowerRead::Scheme(scheme) => self.on_scheme(scheme, now),
                PowerRead::Checked(check) => self.on_check(check, now),
            }
        }
        let guard = &mut self.power_guard;
        if !guard.reader.idle() {
            return;
        }
        let paused = self.paused;
        let job = if guard.cadence.due(now, paused) {
            guard.cadence.checked_at = Some(now);
            guard.cadence.scheme_at = Some(now);
            check::check_job(
                self.changes.journal().clone(),
                self.changes.pending_writes(),
            )
        } else if guard.cadence.scheme_due(now, paused) {
            guard.cadence.scheme_at = Some(now);
            check::scheme_job()
        } else {
            return;
        };
        guard.reader.submit(job);
    }

    /// 电源计划换了：当前模式重新写入新计划，之后勤查一段时间。读不到时不算换了。
    fn on_scheme(&mut self, scheme: Option<String>, now: Millis) {
        let Some(scheme) = scheme else {
            return;
        };
        if self.power_guard.scheme.as_ref() == Some(&scheme) {
            return;
        }
        if let Some(old) = self.power_guard.scheme.replace(scheme.clone()) {
            self.logbook
                .event_with(Event::PlanSwitched, format!("{old} → {scheme}"));
            self.hardware.power_plan_changed();
            self.power_guard.check_soon(now);
        }
    }

    fn on_check(&mut self, check: PowerCheck, now: Millis) {
        self.on_scheme(check.scheme, now);
        self.note_overlay(check.overlay);
        if let Some(scheme) = self.power_guard.scheme.clone() {
            self.restore_drifted(check.drifted, &scheme, now);
        }
        self.complete_power_items(&check.reverted, now);
    }

    /// Poise 在当前计划里写入、又被别处改掉的电源设置：改回来。检查期间日志变了的条目（换了模式、
    /// 别的来源叠在上面）不算；还排在执行线程里的写入检查时当作已经写下（见 `check`）。
    fn restore_drifted(
        &mut self,
        drifted: Vec<(JournalEntry, Option<Value>)>,
        scheme: &str,
        now: Millis,
    ) {
        let journal = self.changes.journal();
        // 暂停着时不守硬件模式写的设置（没撤销成、等着再撤销的那些）：Poise 退出了，不能再把它们改回去。
        let paused = self.paused;
        let drifted: Vec<_> = drifted
            .into_iter()
            .filter(|(e, _)| {
                guarded(e, scheme)
                    && journal.is_current(e)
                    && !(paused && e.owner == poise_hardware::power::OWNER)
            })
            .collect();
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

    /// 用户应用过、现在却不是优化状态的电源项：重新应用（检查期间用户已经还原了的不算）。
    fn complete_power_items(&mut self, reverted: &[String], now: Millis) {
        if reverted.is_empty() {
            return;
        }
        let items: Vec<OptimizationItem> = optimize::power_items()
            .into_iter()
            .filter(|item| reverted.contains(&item.id) && self.changes.journal().owns(&item.id))
            .collect();
        for item in items {
            match self.changes.edit(|s, j| optimize::apply(&item, s, j, now)) {
                Ok(()) => self.logbook.event(Event::OptimizationReapplied {
                    id: item.id.clone(),
                }),
                Err(e) => log::warn!("优化项 {} 已不是优化状态，重新应用失败：{e}", item.id),
            }
        }
    }

    /// 电源模式滑块变化时记一条日志：处于"最佳能效"时，它会盖住计划里的能效偏好、最小处理器状态等。
    fn note_overlay(&mut self, overlay: Option<GUID>) {
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
        let mut g = Cadence::default();
        assert!(g.due(0, false));
        g.checked_at = Some(0);
        assert!(!g.due(5_000, false));
        assert!(g.due(WATCHFUL_EVERY_MS, false));
        g.checked_at = Some(WATCHFUL_FOR_MS);
        assert!(!g.due(WATCHFUL_FOR_MS + WATCHFUL_EVERY_MS, false));
        assert!(g.due(WATCHFUL_FOR_MS + CHECK_EVERY_MS, false));
        // 登录：立即查，之后又勤查一段时间。
        let login = 10 * 60_000;
        g.check_soon(login);
        assert!(g.due(login, false));
        g.checked_at = Some(login);
        assert!(g.due(login + WATCHFUL_EVERY_MS, false));
        // 暂停着：勤查期内照样勤查，之后放慢。
        assert!(g.due(login + WATCHFUL_EVERY_MS, true));
        let later = login + WATCHFUL_FOR_MS;
        g.checked_at = Some(later);
        assert!(!g.due(later + CHECK_EVERY_MS, true));
        assert!(g.due(later + PAUSED_CHECK_EVERY_MS, true));
    }

    #[test]
    fn setting_values_read_as_numbers() {
        assert_eq!(dword(Some(&Value::Dword(2))), Some(2));
        assert_eq!(dword(Some(&Value::String("x".into()))), None);
        assert_eq!(dword(None), None);
    }
}
