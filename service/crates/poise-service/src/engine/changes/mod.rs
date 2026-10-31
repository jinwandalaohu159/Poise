//! 持久修改的唯一出口：设置存储 + 变更日志 + 落盘。
//! 观察模式下拒绝所有修改。电源设置经电源执行线程写入（见 `poise_hardware::PowerWriter`）：
//! 电源服务忙的时候每写一项、重新激活一次电源计划都要几十到几百毫秒，引擎不等它。
//!
//! 先落盘、后写入：一次修改里的写入先记下（注册表、配置文件见 `poise_core::write_ahead`，电源设置见
//! `poise_hardware::PowerWriter`），变更日志落盘之后才写下去、交给执行线程（见 [`Changes::edit`]）；
//! 撤销的写入还排着的时候，落盘的日志仍带上被撤销的条目（见 [`Changes::settle`]）。这样无论在哪一刻断电、
//! 崩溃，下次启动都能按日志回到原值。
//!
//! 这里是对外的修改接口；[`commit`] 是一次修改怎样落到系统上，[`unsettled`] 管撤销还没写完的条目。

mod commit;
mod unsettled;

use std::path::Path;

use poise_core::journal::{Journal, JournalEntry, JournalError, JournalStorage};
use poise_core::model::Millis;
use poise_core::optimize::{self, OptimizationItem};
use poise_core::setting::{Target, Value};
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
        self.resolve_places();
        Ok(())
    }

    /// 旧版本记的位置没写明是哪个电源计划（当时指当前计划）、哪个用户（当时指正在用电脑的用户），
    /// 换成具体的，之后按它还原。
    fn resolve_places(&mut self) {
        if self.journal.resolve_places(&self.store)
            && let Err(e) = self.file.save(self.journal.entries())
        {
            log::error!("保存变更日志失败：{e}");
        }
    }

    /// 当前用户（按用户保存的设置写到哪里）。知道是谁之后，旧版本记的没写明用户的修改归给他
    /// （旧版本只认正在用电脑的那个用户）。
    pub fn set_user(&mut self, user: Option<UserContext>) {
        self.store.set_user(user);
        if self.readable {
            self.resolve_places();
        }
    }

    pub fn user(&self) -> Option<UserContext> {
        self.store.user()
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

    /// 还原某个来源在当前用户和本机上的修改，别的用户的留着（见 [`Journal::here`]）。
    pub fn revert_here(&mut self, owner: &str) -> Result<(), ChangeError> {
        self.edit(|s, j| j.revert_here(s, owner))
    }

    /// 这个来源在当前用户和本机上有修改（见 [`Journal::here`]）。
    pub fn owns_here(&self, owner: &str) -> bool {
        self.journal.owns_here(&self.store, owner)
    }

    /// 这条修改在当前用户和本机上（见 [`Journal::here`]）。
    pub fn here(&self, entry: &JournalEntry) -> bool {
        Journal::here(&self.store, &entry.target)
    }

    /// 把不是 Poise 改的设置恢复为系统默认。不记入日志：回到默认后没有需要还原的。
    pub fn restore_default(&mut self, item: &OptimizationItem) -> Result<(), ChangeError> {
        self.edit(|s, _| optimize::restore_default(item, s).map_err(JournalError::Revert))
    }

    /// 还原当前用户和本机上所有"系统优化"的修改（不含游戏偏好和硬件模式；别的用户的留着）。
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
            if self.owns_here(&owner) && self.revert_here(&owner).is_err() {
                failed.push(owner);
            }
        }
        Ok(failed)
    }

    /// 还原全部修改（卸载时调用），包括别的用户的：他们没登录时临时加载他们的配置单元。即使在观察模式下也
    /// 允许，因为它只会回到原始状态。变更日志读不到时什么都还原不了：报告失败（卸载时据此保留数据目录，
    /// 之后可以重试）。
    pub fn revert_everything(&mut self) -> Vec<String> {
        if !self.readable && self.reload().is_err() {
            return vec!["journal.json".into()];
        }
        self.store.load_absent_users(true);
        let failures = self.journal.revert_all(&self.store);
        if let Err(e) = self.save() {
            log::error!("保存变更日志失败：{e}");
            return vec!["journal.json".into()];
        }
        failures.into_iter().map(|(o, _)| o).collect()
    }
}
