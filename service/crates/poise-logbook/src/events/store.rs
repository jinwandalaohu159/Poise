//! 日志的文件：每条事件一行 JSON，追加到 events.jsonl；写满 [`MAX_BYTES`] 后改名为 events.1.jsonl
//! （覆盖更早的那份）再从头写。读不懂的行（格式变了的旧版本、写到一半断电）跳过。
//!
//! 文件一直开着，一批事件只写一次（游戏开局时调度的决定集中变化，一次几十条）：每条都打开、写入、
//! 关闭的话，关闭被写过的文件还常常引来杀毒软件扫描一遍。

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use poise_protocol::EventEntry;

/// 每个文件最大的字节数（约几千条）。
const MAX_BYTES: u64 = 1024 * 1024;

pub(super) struct EventFile {
    path: PathBuf,
    old: PathBuf,
    /// 打开着的文件，以及它有多大。
    file: Option<(File, u64)>,
    /// 还没写进文件的行。
    pending: Vec<u8>,
}

impl EventFile {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("events.jsonl"),
            old: dir.join("events.1.jsonl"),
            file: None,
            pending: Vec::new(),
        }
    }

    /// 最近的 `n` 条，从早到晚。从文件末尾往前解析，够了就停（启动时不必把两个文件全解析一遍）。
    pub fn load_tail(&self, n: usize) -> Vec<EventEntry> {
        let mut tail = VecDeque::with_capacity(n);
        for path in [&self.path, &self.old] {
            if tail.len() >= n {
                break;
            }
            // 按字节读、逐行解析：断电时写坏的一行（甚至不是合法的 UTF-8）只丢那一行，不丢整个文件。
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            for line in bytes.split(|&b| b == b'\n').rev() {
                if let Ok(entry) = serde_json::from_slice::<EventEntry>(line) {
                    tail.push_front(entry);
                    if tail.len() >= n {
                        break;
                    }
                }
            }
        }
        tail.into()
    }

    /// 追加一条（先放着，[`EventFile::flush`] 时一起写）。
    pub fn append(&mut self, entry: &EventEntry) {
        if serde_json::to_writer(&mut self.pending, entry).is_ok() {
            self.pending.push(b'\n');
        }
    }

    /// 把放着的行写进文件。写不进去时只记文本日志：结构化日志丢几条不影响任何功能。
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        // 写满了：改名为旧文件（直接替换掉更旧的）。改名失败（文件被别的程序占着）时接着往原文件里写，
        // 下一批再试；不先删旧文件，免得改名失败时历史全没了。
        if self.file.as_ref().is_some_and(|(_, len)| *len >= MAX_BYTES) {
            self.file = None;
            if let Err(e) = std::fs::rename(&self.path, &self.old) {
                log::debug!("轮换 events.jsonl 失败（下次再试）：{e}");
            }
        }
        if self.file.is_none() {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                Ok(mut f) => {
                    let mut len = f.metadata().map_or(0, |m| m.len());
                    // 上次写到一半（断电）留下了不完整的一行：先补上换行，不让下一条接在它后面一起坏掉。
                    if len > 0 && !ends_with_newline(&self.path) && f.write_all(b"\n").is_ok() {
                        len += 1;
                    }
                    self.file = Some((f, len));
                }
                Err(e) => log::warn!("打开 events.jsonl 失败：{e}"),
            }
        }
        if let Some((f, len)) = self.file.as_mut() {
            match f.write_all(&self.pending) {
                Ok(()) => *len += self.pending.len() as u64,
                Err(e) => {
                    log::warn!("写入 events.jsonl 失败：{e}");
                    self.file = None;
                }
            }
        }
        self.pending.clear();
    }
}
/// 文件最后一个字节是换行。
fn ends_with_newline(path: &std::path::Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let mut last = [0u8; 1];
    std::fs::File::open(path)
        .and_then(|mut f| {
            f.seek(SeekFrom::End(-1))?;
            f.read_exact(&mut last)
        })
        .is_ok_and(|()| last[0] == b'\n')
}
