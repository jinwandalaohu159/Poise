//! 排队等电源执行线程写入的电源设置（[`PowerQueue`]），以及别的线程读电源设置时用的视图
//! （[`PendingWrites`]：还排着的写入当作已经写下）。执行线程写完一项才把它从队里拿走，所以"先看队里、
//! 再读存储"总能读到写完或将要写的值。

use std::sync::{Arc, Mutex, MutexGuard};

use poise_core::setting::{SettingStore, StoreError, Target, Value};

/// 排队等执行线程写入的电源设置（按先后）。
#[derive(Clone, Default)]
pub(crate) struct PowerQueue(Arc<Mutex<Vec<(Target, Value)>>>);

impl PowerQueue {
    fn lock(&self) -> MutexGuard<'_, Vec<(Target, Value)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn extend(&self, writes: Vec<(Target, Value)>) {
        self.lock().extend(writes);
    }

    /// 这个位置排在队里的最新值（写完之前，它就是"此刻的值"）。
    pub(super) fn latest(&self, target: &Target) -> Option<Value> {
        latest(&self.lock(), target)
    }

    /// 执行线程：取出此刻排着的全部写入（还留在队里）。
    pub fn pending(&self) -> Vec<(Target, Value)> {
        self.lock().clone()
    }

    /// 执行线程：前 `count` 项已经写完，从队里拿走（之后排进来的留着）。写完才拿走：
    /// 别的线程先看队里、再读存储，总能读到写完或将要写的值。
    pub fn done(&self, count: usize) {
        let mut queue = self.lock();
        let count = count.min(queue.len());
        queue.drain(..count);
    }

    /// 排着的都写完了。
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
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
        match self.queue.latest(target) {
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
