//! 记录系统：每段有重点程序的时间汇总成一条（规则见 [`recorder`]），保存在数据目录的 records.json，
//! 最多留 [`KEEP`] 条。一段结束时才写盘，平时不写。

mod recorder;

use std::path::{Path, PathBuf};

use poise_protocol::SessionRecord;

use crate::round::Round;
use recorder::SessionRecorder;

const FILE: &str = "records.json";
/// 最多留多少条（每条几百字节）。
const KEEP: usize = 200;

pub(crate) struct RecordBook {
    recorder: SessionRecorder,
    path: PathBuf,
}

impl RecordBook {
    /// 打开数据目录里的记录，返回它和之前保存的记录（从早到晚）。文件读得到却解析不了时，
    /// 改名为 records.json.bad 留着，不让下一次保存把它覆盖掉。
    pub fn open(dir: &Path) -> (Self, Vec<SessionRecord>) {
        let path = dir.join(FILE);
        let saved = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!("使用记录读不出来（{e}），另存为 records.json.bad");
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        let book = Self {
            recorder: SessionRecorder::default(),
            path,
        };
        (book, saved)
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
