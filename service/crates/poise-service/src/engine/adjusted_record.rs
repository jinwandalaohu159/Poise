//! 记下被调整过的进程（见 `poise_core::schedule::Adjusted`）：服务异常退出（崩溃、被强行结束）
//! 来不及撤销时，下次启动把还在运行的那些进程恢复原样——否则它们会一直留在小核上、一直低优先级，
//! 直到自己重启。正常停止时已经全部撤销，记录是空的。

use std::path::Path;

use poise_core::model::Millis;
use poise_core::schedule::Adjusted;
use poise_platform::persist::storage::JsonFile;
use poise_platform::persist::writer::FileWriter;

/// 记录的文件名（在数据目录里）。
const FILE: &str = "adjusted.json";
/// 有变化时最多这么久保存一次（调整多的时候不每轮写盘）；全部撤销时立即保存。
const SAVE_EVERY_MS: Millis = 30_000;

pub(super) struct AdjustedRecord {
    file: JsonFile<Vec<Adjusted>>,
    writer: FileWriter,
    saved: Vec<Adjusted>,
    saved_at: Option<Millis>,
    /// 上次运行留下的记录，还没处理。
    leftovers: Option<Vec<Adjusted>>,
}

impl AdjustedRecord {
    pub fn open(data_dir: &Path, writer: FileWriter) -> Self {
        let file = JsonFile::new(data_dir.join(FILE));
        let leftovers = file.load().filter(|l: &Vec<Adjusted>| !l.is_empty());
        Self {
            file,
            writer,
            saved: leftovers.clone().unwrap_or_default(),
            saved_at: None,
            leftovers,
        }
    }

    /// 上次运行留下的调整（只给一次）。
    pub fn take_leftovers(&mut self) -> Option<Vec<Adjusted>> {
        self.leftovers.take()
    }

    /// 当前已调整的进程（排好序）。有变化时保存，见 [`SAVE_EVERY_MS`]。
    pub fn update(&mut self, current: Vec<Adjusted>, now: Millis) {
        if current == self.saved {
            return;
        }
        let due = current.is_empty()
            || self
                .saved_at
                .is_none_or(|t| now.saturating_sub(t) >= SAVE_EVERY_MS);
        if due {
            self.save(current, now);
        }
    }

    /// 不等间隔，有变化就保存（撤销完时）。服务停止前由 [`FileWriter::flush`] 等它写完。
    pub fn flush(&mut self, current: Vec<Adjusted>, now: Millis) {
        if current != self.saved {
            self.save(current, now);
        }
    }

    fn save(&mut self, current: Vec<Adjusted>, now: Millis) {
        self.file.save_later(&current, &self.writer);
        self.saved = current;
        self.saved_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use poise_core::model::{PriorityClass, ProcessKey};

    use super::*;

    fn adjusted(pid: u32) -> Adjusted {
        Adjusted {
            key: ProcessKey { pid, created: 1 },
            original_priority: PriorityClass::Normal,
        }
    }

    #[test]
    fn changes_are_saved_at_most_every_few_seconds_and_leftovers_come_back_once() {
        let dir = std::env::temp_dir().join(format!("poise-adjusted-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let writer = FileWriter::start();
        let open = || {
            assert!(writer.flush(std::time::Duration::from_secs(5)));
            AdjustedRecord::open(&dir, writer.clone())
        };
        let mut r = open();
        assert!(r.take_leftovers().is_none());
        r.update(vec![adjusted(1)], 0);
        // 几秒内又变了：先不写盘。
        r.update(vec![adjusted(1), adjusted(2)], 2_000);
        assert_eq!(r.saved, vec![adjusted(1)]);
        r.update(vec![adjusted(1), adjusted(2)], SAVE_EVERY_MS);
        assert_eq!(r.saved.len(), 2);

        // 模拟崩溃：没有撤销就重新打开。
        let mut again = open();
        assert_eq!(again.take_leftovers().map(|l| l.len()), Some(2));
        assert!(again.take_leftovers().is_none());
        // 全部撤销：立即保存为空。
        again.update(Vec::new(), 1);
        assert!(open().take_leftovers().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
