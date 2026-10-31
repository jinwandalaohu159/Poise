//! 记录系统：每段有重点程序的时间汇总成一条（规则见 [`recorder`]），保存在数据目录的 records.json，
//! 最多留 [`KEEP`] 条。一段结束时才写盘，平时不写。

mod recorder;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poise_protocol::SessionRecord;

use crate::round::Round;
use recorder::SessionRecorder;

const FILE: &str = "records.json";
/// 最多留多少条（每条几百字节）。
const KEEP: usize = 200;
/// 读文件一时失败（杀毒软件、备份程序正独占着它）时隔这么久再试，最多试 [`READ_ATTEMPTS`] 次。
const READ_RETRY: Duration = Duration::from_millis(100);
const READ_ATTEMPTS: u32 = 3;

pub(crate) struct RecordBook {
    recorder: SessionRecorder,
    path: PathBuf,
    /// 文件在、却读不到：读到之前不保存，免得把之前的记录覆盖掉（见 [`RecordBook::recover`]）。
    unread: bool,
}

impl RecordBook {
    /// 打开数据目录里的记录，返回它和之前保存的记录（从早到晚）。文件读得到却解析不了时，
    /// 改名（带上时间）留着，不让下一次保存把它覆盖掉；读不到时之后再试（见 [`RecordBook::recover`]）。
    pub fn open(dir: &Path) -> (Self, Vec<SessionRecord>) {
        let path = dir.join(FILE);
        let (saved, unread) = match read(&path) {
            Ok(saved) => (saved, false),
            Err(e) => {
                log::warn!("使用记录读不到（{e}）：读到之前不保存，免得覆盖之前的记录");
                (Vec::new(), true)
            }
        };
        let book = Self {
            recorder: SessionRecorder::default(),
            path,
            unread,
        };
        (book, saved)
    }

    /// 启动时没读到的话再读一次；读到了返回之前保存的记录，之后照常保存。
    pub fn recover(&mut self) -> Option<Vec<SessionRecord>> {
        if !self.unread {
            return None;
        }
        let saved = read(&self.path).ok()?;
        self.unread = false;
        log::info!("使用记录读到了（{} 条）", saved.len());
        Some(saved)
    }

    /// 可以保存（之前的记录读到了，或者本来就没有）。
    pub fn writable(&self) -> bool {
        !self.unread
    }

    /// 喂一轮；一段结束时返回它的记录。
    pub fn observe(&mut self, round: &Round) -> Option<SessionRecord> {
        self.recorder.observe(round)
    }

    /// 结束正在记录的一段。
    pub fn finish(&mut self) -> Option<SessionRecord> {
        self.recorder.finish()
    }

    /// 加上一条记录，去掉最早的多余部分。
    pub fn add(list: &mut Vec<SessionRecord>, record: SessionRecord) {
        list.push(record);
        let excess = list.len().saturating_sub(KEEP);
        list.drain(..excess);
    }

    /// 保存：先写临时文件、落到磁盘上，再替换，写到一半断电不会损坏（一段结束时才保存，不频繁）。
    pub fn save(&self, list: &[SessionRecord]) {
        let tmp = self.path.with_extension("tmp");
        let saved = serde_json::to_vec(list)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let mut file = std::fs::File::create(&tmp)?;
                std::io::Write::write_all(&mut file, &bytes)?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = saved {
            log::warn!("保存使用记录失败：{e}");
        }
    }
}

/// 读记录：文件不存在时是空的；读得到却解析不了时改名留着、当成空的；读不到（被别的程序独占、没有权限）
/// 时隔一会儿再试，还读不到就返回错误——不能当成空的。
fn read(path: &Path) -> io::Result<Vec<SessionRecord>> {
    let mut attempt = 1;
    let bytes = loop {
        match std::fs::read(path) {
            Ok(bytes) => break bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) if attempt >= READ_ATTEMPTS => return Err(e),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(READ_RETRY);
            }
        }
    };
    Ok(serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let aside = path.with_extension(format!("{stamp}.bad"));
        log::warn!("使用记录读不出来（{e}），另存为 {}", aside.display());
        let _ = std::fs::rename(path, &aside);
        Vec::new()
    }))
}
