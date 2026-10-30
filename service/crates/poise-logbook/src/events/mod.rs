//! 日志系统：每条事件（见 `poise_protocol::Event`）编上号，写进文本日志 poise.log（中文，见 [`text`]），
//! 追加到 events.jsonl（见 [`store`]，服务重启后界面还能看到之前的）。最近 [`KEEP`] 条留在内存里给界面看，
//! 界面记住最后一条的编号，之后只取更新的（见 [`page`]）。

mod store;
mod text;

use std::collections::VecDeque;
use std::path::Path;

use poise_protocol::{Event, EventEntry};

use store::EventFile;

/// 内存里留多少条（界面能翻到的最早的）。
pub(crate) const KEEP: usize = 1000;
/// 一次最多返回多少条（取最新的）。
const PAGE: usize = 500;

pub(crate) struct EventLog {
    file: EventFile,
    next_id: u64,
}

impl EventLog {
    /// 打开数据目录里的日志，返回它和之前的最近 [`KEEP`] 条（从早到晚）。
    pub fn open(dir: &Path) -> (Self, VecDeque<EventEntry>) {
        let file = EventFile::new(dir);
        let recent: VecDeque<EventEntry> = file.load_tail(KEEP).into();
        let next_id = recent.back().map_or(1, |e| e.id + 1);
        (Self { file, next_id }, recent)
    }

    /// 记一条：写进文本日志（`detail` 只写在那里，给排查问题的人看）、追加到文件，返回编好号的条目。
    pub fn write(&mut self, at_unix_ms: u64, event: Event, detail: Option<&str>) -> EventEntry {
        let (level, line) = text::describe(&event);
        match detail {
            Some(d) => log::log!(level, "{line}：{d}"),
            None => log::log!(level, "{line}"),
        }
        let entry = EventEntry {
            id: self.next_id,
            at_unix_ms,
            category: event.category(),
            event,
        };
        self.next_id += 1;
        self.file.append(&entry);
        entry
    }
}

/// 编号大于 `after` 的条目，从早到晚，最多 [`PAGE`] 条（多了取最新的）。
pub(crate) fn page(entries: &VecDeque<EventEntry>, after: u64) -> Vec<EventEntry> {
    let newer = entries.iter().filter(|e| e.id > after);
    let skip = newer.clone().count().saturating_sub(PAGE);
    newer.skip(skip).cloned().collect()
}

#[cfg(test)]
mod tests {
    use poise_protocol::EventCategory;

    use super::*;

    #[test]
    fn events_are_numbered_and_come_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("poise-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut log, recent) = EventLog::open(&dir);
        assert!(recent.is_empty());
        let mut kept = VecDeque::new();
        kept.push_back(log.write(1, Event::Resumed, None));
        kept.push_back(log.write(2, Event::MovedAside { count: 3 }, Some("细节")));
        assert_eq!(kept[1].category, EventCategory::Schedule);
        assert_eq!(page(&kept, 0).len(), 2);
        assert!(page(&kept, kept[1].id).is_empty());

        let (mut again, recent) = EventLog::open(&dir);
        assert_eq!(recent.len(), 2);
        assert_eq!(again.write(3, Event::Paused, None).id, kept[1].id + 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_page_holds_the_newest_entries() {
        let entries: VecDeque<EventEntry> = (1..=PAGE as u64 + 10)
            .map(|id| EventEntry {
                id,
                at_unix_ms: id,
                category: EventCategory::System,
                event: Event::Resumed,
            })
            .collect();
        let p = page(&entries, 0);
        assert_eq!(p.len(), PAGE);
        assert_eq!(p.last().map(|e| e.id), Some(PAGE as u64 + 10));
    }
}
