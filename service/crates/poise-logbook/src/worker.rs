//! 日志线程：读回之前保存的日志和记录，然后依次处理交来的事件和每轮摘要。所有写文件都在这里。
//! 所有发送端都放下（服务退出）时结束。

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use poise_protocol::SessionRecord;

use crate::events::{self, EventLog};
use crate::records::RecordBook;
use crate::{Message, Shared};

pub(crate) fn start(dir: PathBuf, rx: Receiver<Message>, shared: Arc<Mutex<Shared>>) {
    let spawned = std::thread::Builder::new()
        .name("logbook".into())
        .spawn(move || run(&dir, &rx, &shared));
    if let Err(e) = spawned {
        log::warn!("启动日志线程失败：{e}");
    }
}

fn run(dir: &std::path::Path, rx: &Receiver<Message>, shared: &Mutex<Shared>) {
    let (mut log, recent) = EventLog::open(dir);
    let (mut book, saved) = RecordBook::open(dir);
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.events = recent;
        s.records = saved;
    }
    for message in rx {
        match message {
            Message::Event {
                at_unix_ms,
                event,
                detail,
            } => {
                let entry = log.write(at_unix_ms, event, detail.as_deref());
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                if s.events.len() >= events::KEEP {
                    s.events.pop_front();
                }
                s.events.push_back(entry);
            }
            Message::Round(round) => {
                if let Some(record) = book.observe(&round) {
                    keep(&book, shared, record);
                }
            }
            Message::EndSession => {
                if let Some(record) = book.finish() {
                    keep(&book, shared, record);
                }
            }
            Message::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// 加上一条记录并保存。写盘在锁外进行，界面读记录不用等磁盘。
fn keep(book: &RecordBook, shared: &Mutex<Shared>, record: SessionRecord) {
    let list = {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        RecordBook::add(&mut s.records, record);
        s.records.clone()
    };
    book.save(&list);
}
