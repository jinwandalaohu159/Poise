//! 排队等电源执行线程写入的电源设置（[`PowerQueue`]），以及别的线程读电源设置时用的视图
//! （[`PendingWrites`]：还排着的写入当作已经写下）。执行线程写完一项才把它从队里拿走，所以"先看队里、
//! 再读存储"总能读到写完或将要写的值。

use std::borrow::Cow;
use std::sync::{Arc, Mutex, MutexGuard};

use poise_core::setting::{SettingStore, StoreError, Target, Value};

/// 排队等执行线程写入的电源设置（按先后），以及写失败了的位置。
#[derive(Clone, Default)]
pub(crate) struct PowerQueue(Arc<Mutex<Queued>>);

#[derive(Default)]
struct Queued {
    writes: Vec<(Target, Value)>,
    /// 写失败的位置（还没被取走）：撤销没写下去时，变更日志要留着这一条（见 `Changes::settle`）。
    failed: Vec<Target>,
}

impl PowerQueue {
    fn lock(&self) -> MutexGuard<'_, Queued> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn extend(&self, writes: Vec<(Target, Value)>) {
        self.lock().writes.extend(writes);
    }

    /// 这个位置排在队里的最新值（写完之前，它就是"此刻的值"）。
    pub(super) fn latest(&self, target: &Target) -> Option<Value> {
        latest(&self.lock().writes, target)
    }

    /// 执行线程：取出此刻排着的全部写入（还留在队里）。
    pub fn pending(&self) -> Vec<(Target, Value)> {
        self.lock().writes.clone()
    }

    /// 执行线程：前 `count` 项已经写过了，从队里拿走（之后排进来的留着），其中写失败的位置记下来。
    /// 写完才拿走：别的线程先看队里、再读存储，总能读到写完或将要写的值。
    pub fn done(&self, count: usize, failed: Vec<Target>) {
        let mut queue = self.lock();
        let count = count.min(queue.writes.len());
        queue.writes.drain(..count);
        queue.failed.extend(failed);
    }

    /// 排着的都写完了。
    pub fn is_empty(&self) -> bool {
        self.lock().writes.is_empty()
    }

    /// 取走写失败的位置。
    pub fn take_failed(&self) -> Vec<Target> {
        std::mem::take(&mut self.lock().failed)
    }
}

/// 队里的位置都写明了计划；没写明的（优化项按"当前计划"）先换成具体的计划再查，否则永远对不上。
pub(super) fn queued_place<'t>(
    target: &'t Target,
    resolve: impl FnOnce(&Target) -> Result<Target, StoreError>,
) -> Cow<'t, Target> {
    match target {
        Target::PowerSetting { scheme: None, .. } => {
            resolve(target).map_or(Cow::Borrowed(target), Cow::Owned)
        }
        _ => Cow::Borrowed(target),
    }
}

pub(super) fn latest(writes: &[(Target, Value)], target: &Target) -> Option<Value> {
    writes
        .iter()
        .rev()
        .find(|(t, _)| t.same_place(target))
        .map(|(_, v)| v.clone())
}

/// 别的线程（电源守护、优化项列表）读电源设置时用：还排在队里的写入当作已经写下（可复制、可跨线程）。
#[derive(Clone)]
pub struct PendingWrites(PowerQueue);

impl PendingWrites {
    pub(crate) fn new(queue: PowerQueue) -> Self {
        Self(queue)
    }

    /// 经过队列读 `store`：队里有的以队里最新的为准，其余照常（写入、删除也照常）。
    pub fn view<'a>(&'a self, store: &'a dyn SettingStore) -> impl SettingStore + 'a {
        PendingView {
            inner: store,
            queue: &self.0,
        }
    }
}

struct PendingView<'a> {
    inner: &'a dyn SettingStore,
    queue: &'a PowerQueue,
}

impl SettingStore for PendingView<'_> {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError> {
        if self.queue.is_empty() {
            return self.inner.read(target);
        }
        let place = queued_place(target, |t| self.inner.resolve(t));
        match self.queue.latest(&place) {
            Some(v) => Ok(Some(v)),
            None => self.inner.read(target),
        }
    }

    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError> {
        self.inner.write(target, value)
    }

    fn delete(&self, target: &Target) -> Result<(), StoreError> {
        self.inner.delete(target)
    }

    fn resolve(&self, target: &Target) -> Result<Target, StoreError> {
        self.inner.resolve(target)
    }

    fn begin_batch(&self) {
        self.inner.begin_batch();
    }

    fn end_batch(&self) {
        self.inner.end_batch();
    }

    fn end_batch_deferred(&self) -> bool {
        self.inner.end_batch_deferred()
    }
}
