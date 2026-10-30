//! 日志的文件：每条事件一行 JSON，追加到 events.jsonl；写满 [`MAX_BYTES`] 后改名为 events.1.jsonl
//! （覆盖更早的那份）再从头写。读不懂的行（格式变了的旧版本、写到一半断电）跳过。

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use poise_protocol::EventEntry;

/// 每个文件最大的字节数（约几千条）。
const MAX_BYTES: u64 = 1024 * 1024;

pub(super) struct EventFile {
    path: PathBuf,
    old: PathBuf,
}

impl EventFile {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("events.jsonl"),
            old: dir.join("events.1.jsonl"),
        }
    }

    /// 最近的 `n` 条，从早到晚。
    pub fn load_tail(&self, n: usize) -> Vec<EventEntry> {
        let mut tail = VecDeque::with_capacity(n + 1);
        for path in [&self.old, &self.path] {
            let Ok(file) = std::fs::File::open(path) else {
                continue;
            };
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(entry) = serde_json::from_str::<EventEntry>(&line) {
                    tail.push_back(entry);
                    if tail.len() > n {
                        tail.pop_front();
                    }
                }
            }
        }
        tail.into()
    }

    /// 追加一条。写不进去时只记文本日志：结构化日志丢一条不影响任何功能。
    pub fn append(&self, entry: &EventEntry) {
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= MAX_BYTES) {
            let _ = std::fs::remove_file(&self.old);
            let _ = std::fs::rename(&self.path, &self.old);
        }
        let Ok(mut line) = serde_json::to_string(entry) else {
            return;
        };
        line.push('\n');
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = written {
            log::warn!("写入 events.jsonl 失败：{e}");
        }
    }
}
