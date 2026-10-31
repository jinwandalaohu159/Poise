//! 先记后写：一次修改里对注册表、配置文件的写入先记下来（之后的读取看得到它们），变更日志落盘之后才真正
//! 写下去（[`Pending::apply`]）。这样写到一半断电、崩溃时，日志里已经有原值。电源设置另有执行线程
//! （同样先落盘后写入，见 `poise_hardware`），直接交给下一层。

use std::cell::RefCell;

use crate::setting::{SettingStore, StoreError, Target, Value};

/// 记下写入、还没写下去的存储。
pub struct WriteAhead<'a> {
    inner: &'a dyn SettingStore,
    ops: RefCell<Vec<Op>>,
}

/// 一项还没写下去的修改：写入一个值，或者删除（None）。位置是具体的（见 [`SettingStore::resolve`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Op {
    target: Target,
    value: Option<Value>,
}

/// 记下的写入，按先后顺序。
#[derive(Debug, Default)]
pub struct Pending(Vec<Op>);

/// 交给执行线程的电源设置不在这里记。
fn deferred(target: &Target) -> bool {
    !matches!(target, Target::PowerSetting { .. })
}

impl<'a> WriteAhead<'a> {
    pub fn new(inner: &'a dyn SettingStore) -> Self {
        Self {
            inner,
            ops: RefCell::default(),
        }
    }

    pub fn into_pending(self) -> Pending {
        Pending(self.ops.into_inner())
    }

    /// 记下一项修改。位置解析不了时（还不知道当前用户）直接交给下一层，由它报错。
    fn record(&self, target: &Target, value: Option<Value>) -> Result<(), StoreError> {
        let target = self.inner.resolve(target)?;
        self.ops.borrow_mut().push(Op { target, value });
        Ok(())
    }
}

impl SettingStore for WriteAhead<'_> {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError> {
        if deferred(target)
            && let Ok(place) = self.inner.resolve(target)
            && let Some(op) = self
                .ops
                .borrow()
                .iter()
                .rev()
                .find(|op| op.target.same_place(&place))
        {
            return Ok(op.value.clone());
        }
        self.inner.read(target)
    }

    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError> {
        if deferred(target) {
            self.record(target, Some(value.clone()))
        } else {
            self.inner.write(target, value)
        }
    }

    fn delete(&self, target: &Target) -> Result<(), StoreError> {
        if deferred(target) {
            self.record(target, None)
        } else {
            self.inner.delete(target)
        }
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

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 按先后写下去，返回没写成的位置和原因（调用方据此把日志里这个位置改回修改前）。
    pub fn apply(self, store: &dyn SettingStore) -> Vec<(Target, StoreError)> {
        let mut failed: Vec<(Target, StoreError)> = Vec::new();
        for op in self.0 {
            // 同一位置前面已经失败了：后面的也不写，免得只写下一半。
            if failed.iter().any(|(t, _)| t.same_place(&op.target)) {
                continue;
            }
            let result = match &op.value {
                Some(v) => store.write(&op.target, v),
                None => store.delete(&op.target),
            };
            if let Err(e) = result {
                failed.push((op.target, e));
            }
        }
        failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;
    use crate::journal::tests::{MemStore, reg};

    #[test]
    fn writes_wait_until_applied_but_reads_see_them() {
        let store = MemStore::default();
        store.write(&reg("a"), &Value::Dword(1)).unwrap();
        let ahead = WriteAhead::new(&store);
        let mut j = Journal::default();
        j.apply(&ahead, "item", &reg("a"), &Value::Dword(0), 1)
            .unwrap();
        // 日志里记下了原值，设置还没动；同一次修改里读到的是要写的值。
        assert_eq!(j.entries()[0].original, Some(Value::Dword(1)));
        assert_eq!(store.read(&reg("a")).unwrap(), Some(Value::Dword(1)));
        assert_eq!(ahead.read(&reg("a")).unwrap(), Some(Value::Dword(0)));
        ahead.delete(&reg("b")).unwrap();
        assert_eq!(ahead.read(&reg("b")).unwrap(), None);
        let pending = ahead.into_pending();
        assert!(pending.apply(&store).is_empty());
        assert_eq!(store.read(&reg("a")).unwrap(), Some(Value::Dword(0)));
    }
}
