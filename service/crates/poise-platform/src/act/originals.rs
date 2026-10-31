//! 调整之前的原值。程序可能自己设过节能（浏览器给后台标签页的进程开效率模式）或核心范围，
//! 撤销调整时一律写"系统默认"会抹掉它自己的设置。所以第一次调整前记下原值；撤销时如果程序在这期间
//! 没有自己改过（读到的还是 Poise 写的），就还原成原值；它自己改过就不动，以它的为准。

use std::collections::HashMap;
use std::sync::Mutex;

use poise_core::model::ProcessKey;

/// 记录多于这么多个进程时，丢掉已经退出的（调整中的进程退出时不会撤销，记录要另外清理）。
/// 清理后还剩很多（都还在运行）时，下次等到翻倍再清理：不每调整一个进程就把所有记录查一遍。
const PRUNE_ABOVE: usize = 512;

/// 撤销调整时怎么办。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo<T> {
    /// 写回这个值。
    Write(T),
    /// 程序在这期间自己改过：不动。
    Leave,
}

struct Entry<T> {
    original: T,
    /// Poise 最近一次写入的值。
    written: Option<T>,
}

struct Records<T> {
    by_key: HashMap<ProcessKey, Entry<T>>,
    /// 记录多于这么多时清理一次（见 [`PRUNE_ABOVE`]）。
    prune_at: usize,
}

/// 一项设置在各进程上的原值。
pub struct Originals<T> {
    records: Mutex<Records<T>>,
}

impl<T> Default for Originals<T> {
    fn default() -> Self {
        Self {
            records: Mutex::new(Records {
                by_key: HashMap::new(),
                prune_at: PRUNE_ABOVE,
            }),
        }
    }
}

impl<T: Clone + PartialEq> Originals<T> {
    /// 调整前调用：第一次调整这个进程时用 `read` 读出原值记下（读不到时按 `fallback`）。
    /// `alive` 用来清理已经退出的进程（记录多了才调用）。
    pub fn remember(
        &self,
        key: ProcessKey,
        read: impl FnOnce() -> Option<T>,
        fallback: T,
        alive: impl Fn(ProcessKey) -> bool,
    ) {
        let mut records = self.lock();
        if records.by_key.contains_key(&key) {
            return;
        }
        if records.by_key.len() >= records.prune_at {
            records.by_key.retain(|k, _| alive(*k));
            records.prune_at = PRUNE_ABOVE.max(records.by_key.len() * 2);
        }
        let original = read().unwrap_or(fallback);
        records.by_key.insert(
            key,
            Entry {
                original,
                written: None,
            },
        );
    }

    /// 调整写入成功后调用。
    pub fn wrote(&self, key: ProcessKey, value: T) {
        if let Some(e) = self.lock().by_key.get_mut(&key) {
            e.written = Some(value);
        }
    }

    /// 撤销调整：`current` 是此刻读到的值。没有记录（例如服务重启后撤销上次留下的调整）时写回 `fallback`。
    pub fn undo(&self, key: ProcessKey, current: Option<T>, fallback: T) -> Undo<T> {
        let Some(entry) = self.lock().by_key.remove(&key) else {
            return Undo::Write(fallback);
        };
        match (current, entry.written) {
            (Some(now), Some(written)) if now != written => Undo::Leave,
            _ => Undo::Write(entry.original),
        }
    }

    /// 进程已经退出：丢掉记录。
    pub fn forget(&self, key: ProcessKey) {
        self.lock().by_key.remove(&key);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Records<T>> {
        self.records.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().by_key.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pid: u32) -> ProcessKey {
        ProcessKey { pid, created: 1 }
    }

    #[test]
    fn untouched_settings_return_to_what_the_program_had() {
        let o = Originals::default();
        o.remember(key(1), || Some(7), 0, |_| true);
        o.wrote(key(1), 3);
        // 再次调整不覆盖原值。
        o.remember(key(1), || Some(3), 0, |_| true);
        o.wrote(key(1), 4);
        assert_eq!(o.undo(key(1), Some(4), 0), Undo::Write(7));
    }

    #[test]
    fn a_setting_the_program_changed_itself_is_left_alone() {
        let o = Originals::default();
        o.remember(key(1), || Some(7), 0, |_| true);
        o.wrote(key(1), 3);
        assert_eq!(o.undo(key(1), Some(5), 0), Undo::Leave);
        // 记录已经丢掉：之后再撤销按默认处理。
        assert_eq!(o.undo(key(1), Some(5), 0), Undo::Write(0));
    }

    #[test]
    fn unreadable_values_fall_back() {
        let o = Originals::default();
        o.remember(key(1), || None, 0, |_| true);
        o.wrote(key(1), 3);
        assert_eq!(o.undo(key(1), None, 0), Undo::Write(0));
    }

    #[test]
    fn exited_processes_are_pruned_when_records_pile_up() {
        let o = Originals::default();
        for pid in 0..PRUNE_ABOVE as u32 {
            o.remember(key(pid), || Some(1), 0, |_| true);
        }
        o.remember(key(9_999), || Some(1), 0, |k| k.pid % 2 == 0);
        assert_eq!(o.len(), PRUNE_ABOVE / 2 + 1);
    }
}
