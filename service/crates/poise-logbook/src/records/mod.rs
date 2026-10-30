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
    /// 打开数据目录里的记录，返回它和之前保存的记录（从早到晚）。
    pub fn open(dir: &Path) -> (Self, Vec<SessionRecord>) {
        let path = dir.join(FILE);
        let saved = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
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

    /// 保存：先写临时文件再替换，写到一半断电不会损坏。
    pub fn save(&self, list: &[SessionRecord]) {
        let tmp = self.path.with_extension("tmp");
        let saved = serde_json::to_vec(list)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = saved {
            log::warn!("保存使用记录失败：{e}");
        }
    }
}
