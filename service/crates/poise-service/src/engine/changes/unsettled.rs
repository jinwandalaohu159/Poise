//! 撤销了、写入还排在电源执行线程里的条目：写完之前落盘时仍带上它们（断电、崩溃后仍能按日志还原），
//! 写完后去掉；撤销没写成的放回日志，原值不丢。

use poise_core::journal::{Journal, JournalEntry, JournalStorage};
use poise_core::setting::Target;
use poise_hardware::PowerWriter;

use super::Changes;

impl Changes {
    /// 这次修改去掉的电源设置条目：撤销的写入写下去之前，落盘时仍带上它们。
    pub(super) fn keep_unsettled(&mut self, before: &Journal) {
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
    pub(super) fn save(&self) -> Result<(), String> {
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
}
