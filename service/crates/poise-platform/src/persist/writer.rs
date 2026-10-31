//! 在后台线程里写文件：调用方只把要写的内容交过来、不等磁盘（每次都要落盘，磁盘忙的时候——
//! 游戏启动、大量读写——一次要几十到几百毫秒）。同一个文件只写最新的一份。
//! 每个文件都先写临时文件、落盘、再替换（见 [`super::atomic::write_atomic`]）。

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::atomic::write_atomic;

#[derive(Default)]
struct State {
    /// 还没写的文件（每个文件只留最新的一份）。
    pending: Vec<(PathBuf, Vec<u8>)>,
    /// 正在写（已经从 `pending` 里拿走）。
    writing: bool,
    /// 句柄都丢弃了：写完剩下的就结束。
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 句柄都丢弃时通知写入线程结束。
struct Handle(Arc<Shared>);

impl Drop for Handle {
    fn drop(&mut self) {
        self.0.lock().closed = true;
        self.0.changed.notify_all();
    }
}

/// 后台写文件（可复制，各处共用一个线程）。
#[derive(Clone)]
pub struct FileWriter(Arc<Handle>);

impl FileWriter {
    pub fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("files".into())
            .spawn(move || run(&worker))
            .expect("创建写文件线程失败");
        Self(Arc::new(Handle(shared)))
    }

    /// 交一份要写的内容；这个文件还有没写的旧内容时替换掉。
    pub fn submit(&self, path: PathBuf, bytes: Vec<u8>) {
        let shared = &self.0.0;
        let mut state = shared.lock();
        match state.pending.iter_mut().find(|(p, _)| *p == path) {
            Some((_, old)) => *old = bytes,
            None => state.pending.push((path, bytes)),
        }
        drop(state);
        shared.changed.notify_all();
    }

    /// 等交过来的都写完（最多 `timeout`），服务停止前调用。返回是否都写完了。
    pub fn flush(&self, timeout: Duration) -> bool {
        let shared = &self.0.0;
        let deadline = Instant::now() + timeout;
        let mut state = shared.lock();
        while !state.pending.is_empty() || state.writing {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }
}

/// 写失败（磁盘满、被独占）后隔这么久再写，同一个文件连续失败最多重试 [`RETRIES`] 次。
const RETRY_AFTER: Duration = Duration::from_secs(2);
const RETRIES: u32 = 3;

fn run(shared: &Shared) {
    // 各文件连续写失败的次数。
    let mut failures: std::collections::HashMap<PathBuf, u32> = Default::default();
    loop {
        let batch = {
            let mut state = shared.lock();
            while state.pending.is_empty() && !state.closed {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if state.pending.is_empty() {
                return;
            }
            state.writing = true;
            std::mem::take(&mut state.pending)
        };
        let mut failed = Vec::new();
        for (path, bytes) in batch {
            match write_atomic(&path, &bytes) {
                Ok(()) => {
                    failures.remove(&path);
                }
                Err(e) => {
                    let count = failures.entry(path.clone()).or_insert(0);
                    *count += 1;
                    log::warn!("保存 {} 失败（第 {count} 次）：{e}", path.display());
                    if *count <= RETRIES {
                        failed.push((path, bytes));
                    } else {
                        failures.remove(&path);
                    }
                }
            }
        }
        if !failed.is_empty() && !shared.lock().closed {
            std::thread::sleep(RETRY_AFTER);
        }
        let mut state = shared.lock();
        // 写失败的放回去再写，除非这期间又交来了更新的内容。
        for (path, bytes) in failed {
            if !state.pending.iter().any(|(p, _)| *p == path) {
                state.pending.push((path, bytes));
            }
        }
        state.writing = false;
        drop(state);
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_content_of_each_file_is_written() {
        let dir = std::env::temp_dir().join(format!("poise-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = FileWriter::start();
        for i in 0..20 {
            w.submit(dir.join("a.json"), format!("{i}").into_bytes());
        }
        w.submit(dir.join("b.json"), b"b".to_vec());
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(std::fs::read(dir.join("a.json")).unwrap(), b"19");
        assert_eq!(std::fs::read(dir.join("b.json")).unwrap(), b"b");
        let _ = std::fs::remove_dir_all(dir);
    }
}
