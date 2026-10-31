//! 电源执行线程：写入排着的电源设置、重新激活电源计划、预读原值。
//!
//! 连续到达的命令合并：排着的电源设置按先后全部写入（后写的覆盖先写的），预读只做最新的一次。
//! 顺序固定为：电源设置 → 重新激活电源计划 → 预读（读到的是写完之后的值）→ 回复等待的人。

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use poise_core::setting::{SettingStore, Target, Value};
use poise_platform::settings::store::WinSettingStore;

use crate::power::{PendingWrites, PowerQueue, Prefetched, Staged, WriteBehind, publish};

/// 写入电源设置超过这么久时记日志。
const SLOW_WRITE: Duration = Duration::from_millis(300);

enum Command {
    /// 队里有新排进的写入（[`PowerQueue`]，位置都是具体的电源计划）。
    Write,
    /// 重新激活电源计划，让已经写入的设置生效。
    Activate,
    /// 读出这些位置此刻的值，放进 [`Prefetched`]（只收这一代的，见 `Prefetched::fill`）。
    Prefetch {
        generation: u64,
        places: Vec<Target>,
    },
    /// 之前排进的都写完后回复。
    Sync(Sender<()>),
}

/// 电源设置的写入口（可复制，调度引擎修改设置的地方各拿一份）：经它修改的电源设置先记下，变更日志落盘后
/// 再发布给执行线程，由执行线程写入并重新激活电源计划（见 `power::WriteBehind`）。
#[derive(Clone)]
pub struct PowerWriter {
    tx: Sender<Command>,
    queue: PowerQueue,
    prefetched: Prefetched,
}

impl PowerWriter {
    pub(super) fn start() -> Self {
        let (tx, rx) = mpsc::channel();
        let writer = Self {
            tx,
            queue: PowerQueue::default(),
            prefetched: Prefetched::default(),
        };
        let (queue, prefetched) = (writer.queue.clone(), writer.prefetched.clone());
        std::thread::Builder::new()
            .name("power-write".into())
            .spawn(move || run(&rx, &queue, &prefetched))
            .expect("创建电源执行线程失败");
        writer
    }

    /// 在 `f` 里经 `store` 修改设置：电源设置的写入先记下（读取依次看记下的、队里还没写完的、预读的），
    /// 其余照常。返回 `f` 的结果与记下的写入：变更日志落盘之后用 [`PowerWriter::commit`] 发布。
    pub fn stage<T>(
        &self,
        store: &dyn SettingStore,
        f: impl FnOnce(&dyn SettingStore) -> T,
    ) -> (T, Staged) {
        let behind = WriteBehind::new(store, &self.queue, &self.prefetched);
        let out = f(&behind);
        (out, behind.into_staged())
    }

    /// 发布记下的写入：排进队列，通知执行线程写入（写完它会重新激活电源计划）。
    pub fn commit(&self, staged: Staged) {
        let command = match publish(&self.queue, &self.prefetched, staged) {
            (true, _) => Command::Write,
            (false, true) => Command::Activate,
            (false, false) => return,
        };
        let _ = self.tx.send(command);
    }

    /// 排着的电源设置都写完了。
    pub fn drained(&self) -> bool {
        self.queue.is_empty()
    }

    /// 取走写失败了的位置（见 `Changes::settle`）。
    pub fn take_failed(&self) -> Vec<Target> {
        self.queue.take_failed()
    }

    /// 别的线程读电源设置时用：还排着的写入当作已经写下（见 [`PendingWrites`]）。
    pub fn pending(&self) -> PendingWrites {
        PendingWrites::new(self.queue.clone())
    }

    pub(crate) fn prefetched(&self) -> &Prefetched {
        &self.prefetched
    }

    /// 读出这些位置此刻的值（预读，见 [`Prefetched`]）。
    pub(crate) fn prefetch(&self, generation: u64, places: Vec<Target>) {
        let _ = self.tx.send(Command::Prefetch { generation, places });
    }

    /// 重新激活电源计划（没经过 [`PowerWriter::stage`] 直接写进了当前计划时）。
    pub(crate) fn activate(&self) {
        let _ = self.tx.send(Command::Activate);
    }

    /// 之前排进的都写完时回复。
    pub(super) fn sync(&self) -> Receiver<()> {
        let (done, finished) = mpsc::channel();
        let _ = self.tx.send(Command::Sync(done));
        finished
    }
}

/// 一批合并后的待办。排着的写入不用记：每批都先把队里的写完。
#[derive(Default)]
struct Pending {
    activate: bool,
    prefetch: Option<(u64, Vec<Target>)>,
    synced: Vec<Sender<()>>,
}

impl Pending {
    fn add(&mut self, command: Command) {
        match command {
            Command::Write => {}
            Command::Activate => self.activate = true,
            Command::Prefetch { generation, places } => self.prefetch = Some((generation, places)),
            Command::Sync(done) => self.synced.push(done),
        }
    }
}

fn run(rx: &Receiver<Command>, queue: &PowerQueue, prefetched: &Prefetched) {
    // 电源设置只写明确的计划，用不到"当前用户"。
    let store = WinSettingStore::new(None);
    while let Ok(first) = rx.recv() {
        let mut pending = Pending::default();
        pending.add(first);
        while let Ok(more) = rx.try_recv() {
            pending.add(more);
        }
        let wrote = write_power(&store, queue);
        if (wrote || pending.activate)
            && let Err(e) = poise_platform::settings::power_plan::activate()
        {
            log::warn!("重新激活电源计划失败：{e:?}");
        }
        // 排着的都写完了：读到的就是此刻的值。
        if let Some((generation, places)) = pending.prefetch {
            prefetched.fill(&store, &places, generation);
        }
        for done in pending.synced {
            let _ = done.send(());
        }
    }
}

/// 按先后写入队里的电源设置（一批，不在这里激活），写完才从队里拿走；返回是否需要重新激活电源计划。
fn write_power(store: &dyn SettingStore, queue: &PowerQueue) -> bool {
    let writes: Vec<(Target, Value)> = queue.pending();
    if writes.is_empty() {
        return false;
    }
    let started = Instant::now();
    store.begin_batch();
    let mut failed = Vec::new();
    for (target, value) in &writes {
        if let Err(e) = store.write(target, value) {
            log::warn!("写入电源设置 {target:?} 失败：{e}");
            failed.push(target.clone());
        }
    }
    let activate = store.end_batch_deferred();
    queue.done(writes.len(), failed);
    if started.elapsed() >= SLOW_WRITE {
        log::info!(
            "写入 {} 项电源设置用了 {} ms（电源服务忙）",
            writes.len(),
            started.elapsed().as_millis()
        );
    }
    activate
}
