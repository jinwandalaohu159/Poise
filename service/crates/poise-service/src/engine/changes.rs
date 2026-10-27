//! 持久修改的唯一出口：设置存储 + 变更日志 + 落盘。
//! 观察模式下拒绝所有修改。

use std::path::Path;

use poise_core::journal::{Journal, JournalError, JournalStorage};
use poise_core::model::Millis;
use poise_core::optimize::{SettingStore, Target, Value};
use poise_platform::persist::storage::FileJournal;
use poise_platform::settings::store::WinSettingStore;
use poise_platform::settings::user::UserContext;

/// 属于"系统优化"的修改（可在优化记录页里全部还原）。其他来源有固定前缀。
pub const GAME_PREFIX: &str = "game:";
pub const HARDWARE_PREFIX: &str = "hardware:";

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
        Self {
            store: WinSettingStore::new(user),
            journal: Journal::from_entries(file.load()),
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
        let result = f(&self.store, &mut self.journal);
        self.file.save(self.journal.entries());
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

    pub fn revert(&mut self, owner: &str) -> Result<(), ChangeError> {
        self.edit(|s, j| j.revert(s, owner))
    }

    /// 还原所有"系统优化"的修改（不含游戏偏好和硬件模式）。
    pub fn revert_optimizations(&mut self) -> Result<Vec<String>, ChangeError> {
        let owners: Vec<String> = self
            .journal
            .entries()
            .iter()
            .map(|e| e.owner.clone())
            .filter(|o| !o.starts_with(GAME_PREFIX) && !o.starts_with(HARDWARE_PREFIX))
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
