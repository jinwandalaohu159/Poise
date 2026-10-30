//! 持久修改的唯一出口：设置存储 + 变更日志 + 落盘。
//! 观察模式下拒绝所有修改。

use std::path::Path;

use poise_core::journal::{Journal, JournalEntry, JournalError, JournalStorage};
use poise_core::model::Millis;
use poise_core::optimize::{self, OptimizationItem};
use poise_core::setting::{SettingStore, Target, Value};
use poise_platform::persist::storage::FileJournal;
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
}

#[derive(Debug)]
pub enum ChangeError {
    DryRun,
    Journal(JournalError),
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DryRun => f.write_str("观察模式下不修改任何设置"),
            Self::Journal(e) => write!(f, "{e}"),
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
        let mut journal = Journal::from_entries(file.load());
        // 旧版本记的电源设置没写明是哪个计划（当时指当前计划）。
        if journal.resolve_places(&store) {
            file.save(journal.entries());
        }
        Self {
            store,
            journal,
            file,
            dry_run,
        }
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// 在可写的前提下修改日志，并立即落盘。
    pub fn edit<T>(
        &mut self,
        f: impl FnOnce(&dyn SettingStore, &mut Journal) -> Result<T, JournalError>,
    ) -> Result<T, ChangeError> {
        if self.dry_run {
            return Err(ChangeError::DryRun);
        }
        let before = self.journal.clone();
        let result = f(&self.store, &mut self.journal);
        // 没有变化时不重写文件（模式没变、电源设置都还在时很常见）。
        if self.journal != before {
            self.file.save(self.journal.entries());
        }
        Ok(result?)
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
    pub fn revert_everything(&mut self) -> Vec<String> {
        let failures = self.journal.revert_all(&self.store);
        self.file.save(self.journal.entries());
        failures.into_iter().map(|(o, _)| o).collect()
    }
}
