//! 持久修改的唯一出口：设置存储 + 变更日志 + 落盘。
//! 观察模式下拒绝所有修改。电源设置经电源执行线程写入（见 `poise_hardware::PowerWriter`）：
//! 电源服务忙的时候每写一项、重新激活一次电源计划都要几十到几百毫秒，引擎不等它。
//!
//! 先落盘、后写入：一次修改里的电源设置先记下，变更日志落盘之后才交给执行线程（见 [`Changes::edit`]）；
//! 撤销的写入还排着的时候，落盘的日志仍带上被撤销的条目（见 [`Changes::settle`]）。这样无论在哪一刻断电、
//! 崩溃，下次启动都能按日志回到原值。

use std::path::Path;

use poise_core::journal::{Journal, JournalEntry, JournalError, JournalStorage};
use poise_core::model::Millis;
use poise_core::optimize::{self, OptimizationItem};
use poise_core::setting::{SettingStore, Target, Value};
use poise_hardware::{PendingWrites, PowerWriter};
use poise_platform::persist::journal_file::FileJournal;
use poise_platform::settings::store::WinSettingStore;
use poise_platform::settings::user::UserContext;

/// 游戏偏好与硬件模式的修改有固定前缀；其余都属于"系统优化"（可在优化记录页里全部还原）。
pub const GAME_PREFIX: &str = "game:";
const HARDWARE_PREFIX: &str = "hardware:";

/// 这项修改是否属于"系统优化"。
pub fn is_optimization(owner: &str) -> bool {
    !owner.starts_with(GAME_PREFIX) && !owner.starts_with(HARDWARE_PREFIX)
}

pub struct Changes {
    pub store: WinSettingStore,
    journal: Journal,
    file: FileJournal,
    dry_run: bool,
    /// 电源设置的写入口；没有时（卸载时还原）直接写。
    power: Option<PowerWriter>,
    /// 撤销了、但撤销的写入还排在执行线程里的条目：写完之前落盘时仍带上（见 [`Changes::save`]）。
    unsettled: Vec<JournalEntry>,
    /// 变更日志读到了（文件不存在也算）。读不到时不修改、不保存，免得覆盖里面的原值。
    readable: bool,
}

#[derive(Debug)]
pub enum ChangeError {
    DryRun,
    Journal(JournalError),
    /// 变更日志读不到或写不进去：没能记下原值的修改不做。
    Unsaved(String),
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DryRun => f.write_str("观察模式下不修改任何设置"),
            Self::Journal(e) => write!(f, "{e}"),
            Self::Unsaved(e) => write!(f, "变更日志读不到或写不进去，没有修改：{e}"),
        }
    }
}

impl From<JournalError> for ChangeError {
    fn from(e: JournalError) -> Self {
        Self::Journal(e)
    }
}

impl Changes {
    pub fn new(data_dir: &Path, user: Option<UserContext>, dry_run: bool) -> Self {
        let file = FileJournal::new(data_dir);
        let store = WinSettingStore::new(user);
        let mut changes = Self {
            store,
            journal: Journal::default(),
            file,
            dry_run,
            power: None,
            unsettled: Vec::new(),
            readable: false,
        };
        if let Err(e) = changes.reload() {
            log::error!("读不到变更日志（{e}）：读到之前不做任何修改，免得覆盖里面的原值");
        }
        changes
    }

    /// 从文件读回日志。读不到（文件在、却打不开）时日志不可用：不修改、不保存（见 [`Changes::edit`]）。
    fn reload(&mut self) -> Result<(), String> {
        let entries = self.file.load()?;
        self.journal = Journal::from_entries(entries);
        self.readable = true;
        // 旧版本记的电源设置没写明是哪个计划（当时指当前计划）。
        if self.journal.resolve_places(&self.store)
            && let Err(e) = self.file.save(self.journal.entries())
        {
            log::error!("保存变更日志失败：{e}");
        }
        Ok(())
    }

    /// 电源设置经硬件执行线程写入。
    pub fn with_power(mut self, power: PowerWriter) -> Self {
        self.power = Some(power);
        self
    }

    /// 别的线程读电源设置时用：还排着的写入当作已经写下（电源守护、优化项列表，见 [`PendingWrites`]）。
    pub fn pending_writes(&self) -> Option<PendingWrites> {
        self.power.as_ref().map(PowerWriter::pending)
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// 在可写的前提下修改日志，并立即落盘。电源设置的写入在落盘之后才交给执行线程
    /// （出错时已经记下的部分照样落盘、写入）。
    pub fn edit<T>(
        &mut self,
        f: impl FnOnce(&dyn SettingStore, &mut Journal) -> Result<T, JournalError>,
    ) -> Result<T, ChangeError> {
        if self.dry_run {
            return Err(ChangeError::DryRun);
        }
        if !self.readable {
            // 启动时没读到：再读一次，读到了才能改。
            self.reload().map_err(ChangeError::Unsaved)?;
            log::info!("变更日志读到了，恢复修改");
        }
        let before = self.journal.clone();
        let journal = &mut self.journal;
        let (result, staged) = match &self.power {
            Some(power) => {
                let (result, staged) = power.stage(&self.store, |s| f(s, journal));
                (result, Some(staged))
            }
            None => (f(&self.store, journal), None),
        };
        if staged.as_ref().is_some_and(|s| s.writes_power()) {
            self.keep_unsettled(&before);
        }
        // 没有变化时不重写文件（模式没变、电源设置都还在时很常见）。
        if self.journal != before
            && let Err(e) = self.save()
        {
            // 没落盘：电源设置还没写（在暂存里），不写了，日志里电源的部分也回到修改前；别的设置已经
            // 写下去了，它们的条目留在内存里（还能还原），下次落盘时一起保存。
            log::error!("保存变更日志失败：{e}");
            self.journal = Self::without_power_changes(&self.journal, &before);
            return Err(ChangeError::Unsaved(e));
        }
        if let (Some(power), Some(staged)) = (&self.power, staged) {
            power.commit(staged);
        }
        Ok(result?)
    }

    /// `now` 里电源设置以外的条目，加上 `before` 里的电源设置条目。
    fn without_power_changes(now: &Journal, before: &Journal) -> Journal {
        let is_power = |e: &&JournalEntry| matches!(e.target, Target::PowerSetting { .. });
        let mut entries: Vec<JournalEntry> = now
            .entries()
            .iter()
            .filter(|e| !is_power(e))
            .cloned()
            .collect();
        entries.extend(before.entries().iter().filter(is_power).cloned());
        Journal::from_entries(entries)
    }

    /// 这次修改去掉的电源设置条目：撤销的写入写下去之前，落盘时仍带上它们。
    fn keep_unsettled(&mut self, before: &Journal) {
        for old in before.entries() {
            let same = |e: &JournalEntry| e.owner == old.owner && e.target.same_place(&old.target);
            let removed = matches!(old.target, Target::PowerSetting { .. })
                && !self.journal.entries().iter().any(same);
            if removed && !self.unsettled.iter().any(same) {
                self.unsettled.push(old.clone());
            }
        }
    }

    /// 排着的电源设置都写下去了：落盘的日志不再带上已经撤销的条目。每轮调用。
    /// 撤销没写成的（电源服务出错、计划被删）放回日志：原值不能丢，之后还原时再试（和 0.7.0 一样）。
    pub fn settle(&mut self) {
        if !self.power.as_ref().is_none_or(PowerWriter::drained) {
            return;
        }
        let failed = self
            .power
            .as_ref()
            .map(PowerWriter::take_failed)
            .unwrap_or_default();
        if self.unsettled.is_empty() {
            return;
        }
        let entries = self.journal.entries();
        let kept: Vec<JournalEntry> = std::mem::take(&mut self.unsettled)
            .into_iter()
            .filter(|u| {
                failed.iter().any(|f| f.same_place(&u.target))
                    && !entries.iter().any(|e| e.target.same_place(&u.target))
            })
            .collect();
        if !kept.is_empty() {
            log::warn!(
                "{} 项电源设置没能还原，留在变更日志里，之后再试",
                kept.len()
            );
            let mut all = entries.to_vec();
            all.extend(kept);
            self.journal = Journal::from_entries(all);
        }
        if let Err(e) = self.save() {
            // 文件里还是带着撤销条目的旧内容：照样能还原，下次落盘时再去掉。
            log::warn!("保存变更日志失败：{e}");
        }
    }

    /// 落盘。撤销还没写下去的条目接在后面（这个位置又有了新条目的以日志为准：
    /// 重新接管时读到的原值就是撤销之后的值）。
    fn save(&self) -> Result<(), String> {
        if !self.readable {
            return Err("变更日志读不到".into());
        }
        let entries = self.journal.entries();
        let unsettled: Vec<&JournalEntry> = self
            .unsettled
            .iter()
            .filter(|u| !entries.iter().any(|e| e.target.same_place(&u.target)))
            .collect();
        if unsettled.is_empty() {
            return self.file.save(entries);
        }
        let mut all = entries.to_vec();
        all.extend(unsettled.into_iter().cloned());
        self.file.save(&all)
    }

    pub fn apply(
        &mut self,
        owner: &str,
        target: &Target,
        value: &Value,
        now: Millis,
    ) -> Result<(), ChangeError> {
        self.edit(|s, j| j.apply(s, owner, target, value, now))
    }

    /// 重新写入被别处改掉的修改（一批写入，电源计划只重新激活一次）。
    pub fn reassert(&mut self, entries: &[JournalEntry], now: Millis) -> Result<(), ChangeError> {
        self.edit(|s, j| {
            s.begin_batch();
            let result = entries
                .iter()
                .try_for_each(|e| j.apply(s, &e.owner, &e.target, &e.applied, now));
            s.end_batch();
            result
        })
    }

    pub fn revert(&mut self, owner: &str) -> Result<(), ChangeError> {
        self.edit(|s, j| j.revert(s, owner))
    }

    /// 把不是 Poise 改的设置恢复为系统默认。不记入日志：回到默认后没有需要还原的。
    pub fn restore_default(&mut self, item: &OptimizationItem) -> Result<(), ChangeError> {
        self.edit(|s, _| optimize::restore_default(item, s).map_err(JournalError::Revert))
    }

    /// 还原所有"系统优化"的修改（不含游戏偏好和硬件模式）。
    pub fn revert_optimizations(&mut self) -> Result<Vec<String>, ChangeError> {
        let owners: Vec<String> = self
            .journal
            .entries()
            .iter()
            .map(|e| e.owner.clone())
            .filter(|o| is_optimization(o))
            .collect();
        let mut failed = Vec::new();
        for owner in owners {
            if self.journal.owns(&owner) && self.revert(&owner).is_err() {
                failed.push(owner);
            }
        }
        Ok(failed)
    }

    /// 还原全部修改（卸载时调用）。即使在观察模式下也允许，因为它只会回到原始状态。
    /// 变更日志读不到时什么都还原不了：报告失败（卸载时据此保留数据目录，之后可以重试）。
    pub fn revert_everything(&mut self) -> Vec<String> {
        if !self.readable && self.reload().is_err() {
            return vec!["journal.json".into()];
        }
        let failures = self.journal.revert_all(&self.store);
        if let Err(e) = self.save() {
            log::error!("保存变更日志失败：{e}");
            return vec!["journal.json".into()];
        }
        failures.into_iter().map(|(o, _)| o).collect()
    }
}
