//! 一次修改怎样落到系统上：先在暂存里改（注册表、配置文件见 `poise_core::write_ahead`，电源设置见
//! `poise_hardware::PowerWriter`），变更日志落盘之后才写下去、交给执行线程；没写成的位置日志改回修改前。

use poise_core::journal::{Journal, JournalError};
use poise_core::setting::{SettingStore, Target};
use poise_core::write_ahead::{Pending, WriteAhead};

use super::{ChangeError, Changes};

impl Changes {
    /// 在可写的前提下修改日志，并立即落盘；落盘之后才写下设置、把电源设置交给执行线程（出错时已经记下的
    /// 部分照样落盘、写入）。没写成的位置，日志改回修改前的样子。
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
        let run = |s: &dyn SettingStore| {
            let ahead = WriteAhead::new(s);
            let result = f(&ahead, journal);
            (result, ahead.into_pending())
        };
        let ((result, pending), staged) = match &self.power {
            Some(power) => {
                let (out, staged) = power.stage(&self.store, run);
                (out, Some(staged))
            }
            None => (run(&self.store), None),
        };
        if staged.as_ref().is_some_and(|s| s.writes_power()) {
            self.keep_unsettled(&before);
        }
        // 没有变化时不重写文件（模式没变、电源设置都还在时很常见）。
        if self.journal != before
            && let Err(e) = self.save()
        {
            // 没落盘：什么都还没写（都在暂存里），不写了，日志回到修改前。
            log::error!("保存变更日志失败：{e}");
            self.journal = before;
            return Err(ChangeError::Unsaved(e));
        }
        if let (Some(power), Some(staged)) = (&self.power, staged) {
            power.commit(staged);
        }
        self.write_pending(pending, &before)?;
        Ok(result?)
    }

    /// 落盘之后把记下的注册表、配置文件写下去。没写成的位置，日志改回修改前（设置没变，日志不能说变了），
    /// 再落盘一次；返回第一个失败。
    fn write_pending(&mut self, pending: Pending, before: &Journal) -> Result<(), ChangeError> {
        if pending.is_empty() {
            return Ok(());
        }
        let failed = pending.apply(&self.store);
        let Some((_, first)) = failed.first().cloned() else {
            return Ok(());
        };
        let count = |j: &Journal, place: &Target| {
            j.entries()
                .iter()
                .filter(|e| e.target.same_place(place))
                .count()
        };
        // 撤销时少了条目、应用时多了条目：据此说是还原失败还是写入失败。
        let reverting = failed
            .first()
            .is_some_and(|(t, _)| count(&self.journal, t) < count(before, t));
        for (place, e) in &failed {
            log::warn!("写入 {place:?} 失败：{e}");
            self.journal.restore_place(before, place);
        }
        if let Err(e) = self.save() {
            log::error!("保存变更日志失败：{e}");
        }
        Err(ChangeError::Journal(if reverting {
            JournalError::Revert(first)
        } else {
            JournalError::Write(first)
        }))
    }
}
