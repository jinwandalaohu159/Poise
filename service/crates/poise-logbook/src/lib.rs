//! 日志系统与记录系统，独立于调度，在自己的线程里工作。
//!
//! - **日志**（[`events`]）：Poise 做的每一件值得知道的事记成一条结构化事件，写进文本日志 poise.log，
//!   追加到 events.jsonl，界面按类别查看。
//! - **记录**（[`records`]）：每一段有重点程序的时间（玩游戏、编译、渲染……）汇总成一条使用记录。
//!
//! 和调度之间只有接口：调度通过 [`Logbook`] 交来事件（[`Logbook::event`]）和每轮的摘要（[`Logbook::round`]，
//! 见 [`Round`]），这些调用只是放进通道，不等待、不碰磁盘，不拖慢调度。格式化、写文件、汇总都在日志线程里做，
//! 这个线程以服务进程默认的低优先级运行。界面要看日志和记录时，直接读 [`Logbook::events_after`]、
//! [`Logbook::records`]（日志线程维护的最近结果），不经过调度引擎、不等它的锁。

mod events;
mod records;
mod round;
mod worker;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poise_protocol::{Event, EventEntry, EventPage, SessionRecord};

pub use round::{FocusProgram, Round};

/// 服务停止时最多等日志线程这么久把剩下的写完。
const FLUSH_TIMEOUT: Duration = Duration::from_secs(3);
/// 最多积压多少条没处理的（日志线程被磁盘卡住时不越积越多，满了丢掉新来的：日志丢几条不影响任何功能）。
const BACKLOG: usize = 8192;

/// 交给日志线程的东西。
enum Message {
    Event {
        at_unix_ms: u64,
        event: Event,
        detail: Option<String>,
    },
    Round(Round),
    /// 结束正在记录的一段（Poise 暂停、服务停止）。
    EndSession,
    /// 之前交来的都处理完了就回一声。
    Flush(Sender<()>),
}

/// 日志线程维护、界面读取的最近结果。
#[derive(Default)]
struct Shared {
    /// 最近的日志，从早到晚。
    events: VecDeque<EventEntry>,
    /// 全部使用记录，从早到晚。
    records: Vec<SessionRecord>,
}

/// 日志系统与记录系统的接口。可以复制，调度引擎、命名管道服务端各拿一份。
#[derive(Clone)]
pub struct Logbook {
    tx: SyncSender<Message>,
    shared: Arc<Mutex<Shared>>,
    dir: PathBuf,
}

impl Logbook {
    /// 在数据目录里打开日志与记录，启动日志线程（之前保存的在日志线程里读回）。
    pub fn open(dir: &Path) -> Self {
        let (tx, rx) = mpsc::sync_channel(BACKLOG);
        let shared = Arc::new(Mutex::new(Shared::default()));
        worker::start(dir.to_path_buf(), rx, shared.clone());
        Self {
            tx,
            shared,
            dir: dir.to_path_buf(),
        }
    }

    /// 记一条事件。
    pub fn event(&self, event: Event) {
        self.send_event(event, None);
    }

    /// 记一条事件；`detail` 只写进文本日志，给排查问题的人看（例如切换模式时写入了哪些设置、用了多久）。
    pub fn event_with(&self, event: Event, detail: String) {
        self.send_event(event, Some(detail));
    }

    /// 交来一轮调度的摘要（记录系统据此汇总）。
    pub fn round(&self, round: Round) {
        let _ = self.tx.try_send(Message::Round(round));
    }

    /// 结束正在记录的一段（Poise 暂停、服务停止时）。
    pub fn end_session(&self) {
        let _ = self.tx.try_send(Message::EndSession);
    }

    /// 等日志线程把之前交来的都写完（服务停止前），最多等 [`FLUSH_TIMEOUT`]。
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.tx.try_send(Message::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(FLUSH_TIMEOUT);
        }
    }

    /// 编号大于 `after` 的日志（`after` 为 0 时取最新的一批），从早到晚。
    pub fn events_after(&self, after: u64) -> EventPage {
        EventPage {
            entries: events::page(&self.lock().events, after),
            log_dir: self.dir.display().to_string(),
        }
    }

    /// 全部使用记录，最新的在前。
    pub fn records(&self) -> Vec<SessionRecord> {
        self.lock().records.iter().rev().cloned().collect()
    }

    fn send_event(&self, event: Event, detail: Option<String>) {
        let _ = self.tx.try_send(Message::Event {
            at_unix_ms: unix_now_ms(),
            event,
            detail,
        });
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use poise_core::model::ProcessKey;
    use poise_core::prefs::Mode;

    use super::*;

    fn round(unix_ms: u64, focused: bool) -> Round {
        Round {
            unix_ms,
            focused,
            focus: vec![FocusProgram {
                key: ProcessKey { pid: 7, created: 1 },
                name: "渲染".into(),
                path: None,
                game: false,
                load_pct: 400,
                domains: 0b11,
            }],
            mode: Mode::Full,
            cpu_pct: 60.0,
            ..Default::default()
        }
    }

    #[test]
    fn events_and_records_are_written_by_the_logbook_thread_and_read_back() {
        let dir = std::env::temp_dir().join(format!("poise-logbook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let book = Logbook::open(&dir);
        book.event(Event::Resumed);
        book.event_with(
            Event::ModeChanged {
                mode: Mode::Full,
                on_battery: false,
            },
            "写入 3 毫秒".into(),
        );
        for i in 0..=30 {
            book.round(round(1_000_000 + i * 2000, true));
        }
        book.end_session();
        book.flush();
        let page = book.events_after(0);
        assert_eq!(page.entries.len(), 2);
        assert!(book.events_after(page.entries[1].id).entries.is_empty());
        let records = book.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].programs[0].name, "渲染");

        // 重新打开：之前的日志和记录都读得回来。
        let again = Logbook::open(&dir);
        again.flush();
        assert_eq!(again.events_after(0).entries.len(), 2);
        assert_eq!(again.records().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
