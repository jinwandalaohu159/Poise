//! 记下被用户直接用过的程序（见 `poise_core::schedule::trackers::work`）：Poise 更新、服务重启后，之前从终端、
//! 编辑器里启动、还在后台跑着的训练和编译照样认得出来，不用等用户再点一下它们。只认还在运行的同一个进程
//! （进程号加创建时间）。

use std::path::Path;

use poise_core::model::{Millis, ProcessKey};
use poise_platform::persist::storage::JsonFile;
use poise_platform::persist::writer::FileWriter;

/// 记录的文件名（在数据目录里）。
const FILE: &str = "engaged.json";
/// 有变化时最多这么久保存一次。
const SAVE_EVERY_MS: Millis = 60_000;

pub(super) struct EngagedRecord {
    file: JsonFile<Vec<ProcessKey>>,
    writer: FileWriter,
    saved: Vec<ProcessKey>,
    saved_at: Option<Millis>,
    /// 上次比较时调度里的记录变过几次（见 `Scheduler::engaged_revision`）：没变就不取、不比较。
    seen_revision: Option<u64>,
    /// 上次运行留下的记录，还没交给调度。
    loaded: Option<Vec<ProcessKey>>,
}

impl EngagedRecord {
    pub fn open(data_dir: &Path, writer: FileWriter) -> Self {
        let file = JsonFile::new(data_dir.join(FILE));
        let loaded: Option<Vec<ProcessKey>> = file.load();
        Self {
            file,
            writer,
            saved: loaded.clone().unwrap_or_default(),
            saved_at: None,
            seen_revision: None,
            loaded,
        }
    }

    /// 上次运行留下的记录（只给一次）。
    pub fn take_loaded(&mut self) -> Option<Vec<ProcessKey>> {
        self.loaded.take()
    }

    /// 调度里的记录变过 `revision` 次，`current` 取出当前的记录（排好序）。有变化时最多每
    /// [`SAVE_EVERY_MS`] 保存一次；没变化时不取。
    pub fn update(
        &mut self,
        revision: u64,
        current: impl FnOnce() -> Vec<ProcessKey>,
        now: Millis,
    ) {
        if self.seen_revision == Some(revision)
            || self
                .saved_at
                .is_some_and(|t| now.saturating_sub(t) < SAVE_EVERY_MS)
        {
            return;
        }
        self.seen_revision = Some(revision);
        let current = current();
        if current == self.saved {
            return;
        }
        self.file.save_later(&current, &self.writer);
        self.saved = current;
        self.saved_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_survives_a_restart_and_is_handed_over_once() {
        let dir = std::env::temp_dir().join(format!("poise-engaged-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let writer = FileWriter::start();
        let key = ProcessKey { pid: 7, created: 9 };
        let mut r = EngagedRecord::open(&dir, writer.clone());
        assert!(r.take_loaded().is_none());
        r.update(1, || vec![key], 0);
        // 一分钟内又变了：先不写盘。
        r.update(2, Vec::new, 1_000);
        assert_eq!(r.saved, vec![key]);
        // 没变过：不取。
        r.update(1, || unreachable!(), SAVE_EVERY_MS);
        assert!(writer.flush(std::time::Duration::from_secs(5)));
        let mut again = EngagedRecord::open(&dir, writer.clone());
        assert_eq!(again.take_loaded(), Some(vec![key]));
        assert!(again.take_loaded().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
