//! 守护要读的东西（在电源设置读取线程里运行，见 [`crate::power_reader`]）：只读，不写。
//! 每轮看一眼当前电源计划；到时间时按变更日志的副本做一次完整检查。

use poise_core::journal::{Journal, JournalEntry};
use poise_core::optimize;
use poise_core::setting::{SettingStore, Target, Value};
use poise_hardware::PendingWrites;
use poise_platform::settings::power_plan;
use windows::core::GUID;

use super::super::changes::GAME_PREFIX;
use crate::power_reader::Job;

/// 读取线程交回的结果。
pub(super) enum PowerRead {
    /// 当前电源计划；读不到时为空。
    Scheme(Option<String>),
    /// 一次完整检查。
    Checked(PowerCheck),
}

pub(super) struct PowerCheck {
    pub scheme: Option<String>,
    /// Windows 电源模式滑块的覆盖方案（没有覆盖时为空）。
    pub overlay: Option<GUID>,
    /// Poise 在当前计划里写入、又被别处改掉的设置，以及它此刻的值。按检查时日志的副本找出，
    /// 用之前要确认条目仍然有效（见 `Journal::is_current`）。
    pub drifted: Vec<(JournalEntry, Option<Value>)>,
    /// 用户应用过、现在却不是优化状态的电源优化项。
    pub reverted: Vec<String>,
}

/// 看一眼当前电源计划。
pub(super) fn scheme_job() -> Job<PowerRead> {
    Box::new(|_| PowerRead::Scheme(active_scheme()))
}

/// 一次完整检查（按日志的副本）：当前计划、滑块、被改掉的设置、不再是优化状态的电源项。
/// `pending`：还排在电源执行线程里的写入，读的时候当作已经写下。
pub(super) fn check_job(journal: Journal, pending: Option<PendingWrites>) -> Job<PowerRead> {
    Box::new(move |store| match &pending {
        Some(pending) => check(&pending.view(store), &journal),
        None => check(store, &journal),
    })
}

fn check(store: &dyn SettingStore, journal: &Journal) -> PowerRead {
    let scheme = active_scheme();
    let drifted = scheme
        .as_deref()
        .map_or_else(Vec::new, |s| journal.drifted(store, |e| guarded(e, s)));
    let items = optimize::power_items();
    let reverted = optimize::reverted_power_items(&items, store, journal)
        .into_iter()
        .map(|item| item.id.clone())
        .collect();
    PowerRead::Checked(PowerCheck {
        scheme,
        overlay: power_plan::effective_overlay(),
        drifted,
        reverted,
    })
}

/// 守护的修改：当前计划里的（游戏偏好不在电源计划里，也不守）。
pub(super) fn guarded(e: &JournalEntry, scheme: &str) -> bool {
    !e.owner.starts_with(GAME_PREFIX) && in_scheme(&e.target, scheme)
}

fn active_scheme() -> Option<String> {
    power_plan::active_scheme()
        .ok()
        .map(|g| power_plan::guid_text(&g))
}

fn in_scheme(target: &Target, active: &str) -> bool {
    matches!(target, Target::PowerSetting { scheme: Some(s), .. } if s.eq_ignore_ascii_case(active))
}

#[cfg(test)]
mod tests {
    use poise_core::power_settings;

    use super::*;

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
        let entry = |owner: &str| JournalEntry {
            owner: owner.into(),
            target: t(Some("abc")),
            original: None,
            applied: Value::Dword(2),
            at: 0,
        };
        assert!(guarded(&entry("hardware:power"), "abc"));
        assert!(!guarded(&entry("game:x"), "abc"));
    }
}
