//! 写入线程与全局日志对象：写日志的线程把格式化好的一行放进有界队列就返回；队列满了（写入线程被饿着、
//! 磁盘卡住）就丢掉这一行并计数，之后在日志里注明丢了多少行，绝不让写日志的线程等待。

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

use log::{LevelFilter, Log, Metadata, Record};

use super::line;
use super::rotating::RotatingFile;

/// 最多排队这么多行。
const QUEUE: usize = 4096;
/// 等写入线程写完最多等这么久。
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

enum Message {
    Line(String),
    /// 之前交来的都写完了就回一声。
    Flush(SyncSender<()>),
}

pub(super) struct ChannelLogger {
    level: LevelFilter,
    tx: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

/// 启动写入线程，返回交给它的日志对象。
pub(super) fn start(
    file: Option<RotatingFile>,
    to_terminal: bool,
    level: LevelFilter,
) -> ChannelLogger {
    let (tx, rx) = sync_channel(QUEUE);
    let dropped = Arc::new(AtomicU64::new(0));
    let counter = dropped.clone();
    let spawned = std::thread::Builder::new()
        .name("log".into())
        .spawn(move || run(rx, file, to_terminal, &counter));
    if spawned.is_err() {
        eprintln!("无法创建日志线程");
    }
    ChannelLogger { level, tx, dropped }
}

impl Log for ChannelLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = line::format(record.level(), record.args());
        if let Err(TrySendError::Full(_)) = self.tx.try_send(Message::Line(line)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        let (done, wait) = sync_channel(1);
        if self.tx.try_send(Message::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(FLUSH_TIMEOUT);
        }
    }
}

/// 写入线程：一次取完排队的行，拼起来只写一次文件。
fn run(
    rx: Receiver<Message>,
    mut file: Option<RotatingFile>,
    to_terminal: bool,
    dropped: &AtomicU64,
) {
    let mut batch = Vec::with_capacity(8 * 1024);
    let mut waiting = Vec::new();
    while let Ok(first) = rx.recv() {
        for message in std::iter::once(first).chain(rx.try_iter()) {
            match message {
                Message::Line(line) => batch.extend_from_slice(line.as_bytes()),
                Message::Flush(done) => waiting.push(done),
            }
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            let note = line::format(
                log::Level::Warn,
                &format_args!("日志太多，丢掉了 {lost} 行"),
            );
            batch.extend_from_slice(note.as_bytes());
        }
        if !batch.is_empty() {
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(&batch);
            }
            if to_terminal {
                let _ = std::io::stderr().write_all(&batch);
            }
            batch.clear();
        }
        for done in waiting.drain(..) {
            let _ = done.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_written_by_the_writer_thread_and_flush_waits_for_them() {
        let dir = std::env::temp_dir().join(format!("poise-log-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.log");
        let logger = start(
            RotatingFile::open(path.clone(), 1024 * 1024),
            false,
            LevelFilter::Info,
        );
        logger.log(
            &Record::builder()
                .level(log::Level::Info)
                .args(format_args!("第一行"))
                .build(),
        );
        logger.log(
            &Record::builder()
                .level(log::Level::Debug)
                .args(format_args!("不写"))
                .build(),
        );
        logger.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with(" [INFO] 第一行\n"), "{text}");
        assert!(!text.contains("不写"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
