//! 电源设置的延后写入：变更日志照常记录，真正写进电源服务的那一步交给电源执行线程，和重新激活电源计划
//! 一起按顺序做。电源服务忙的时候（游戏刚启动、刚切换计划）每写一项要几十到几百毫秒，切换一次模式要写
//! 好几项（实测在调度线程里一卡就是 1～2.5 秒），调度不再等它。所有电源设置的修改（模式、系统优化项、
//! 电源守护改回）都走这里（见 `PowerWriter`），重新激活电源计划只在执行线程里做。
//!
//! 先记下、后发布：一次修改里的电源设置写入先留在 [`WriteBehind`] 里（[`Staged`]），等变更日志落盘之后
//! 才排进队列（见 `queue`）交给执行线程——否则执行线程可能抢在日志落盘之前写下去，这时断电或崩溃，
//! 设置改了、日志里却没有，就再也还原不了。
//!
//! 排队的写入在执行线程写完之前一直留在队里：这期间读"此刻的值"以队里最新的为准（电源守护和优化项
//! 列表在别的线程里读，也经过它，见 `queue::PendingWrites`）。否则快速连续切换模式时（全力 → 均衡 → 全力），
//! 第二次接管某个设置读到的"原值"可能是执行线程刚写入的 Poise 自己的值，退出后就回不到用户原来的设置了。
//! 队里没有的，再看执行线程预先读好的（见 [`Prefetched`]）。
//!
//! 写入失败由执行线程记日志；变更日志里的值仍是要写的值，电源守护发现不一致时会改回。

use std::cell::{Cell, RefCell};

use poise_core::setting::{SettingStore, StoreError, Target, Value};

use super::prefetch::Prefetched;
use super::queue::{PowerQueue, latest, queued_place};

/// 一次修改里记下、还没发布的电源设置写入（见 `PowerWriter::commit`）。
#[derive(Default)]
#[must_use = "要在变更日志落盘之后发布"]
pub struct Staged {
    pub(crate) writes: Vec<(Target, Value)>,
    /// 一批写入结束时要重新激活电源计划（交给执行线程，不在这里激活）。
    pub(crate) activate: bool,
}

impl Staged {
    /// 记下了电源设置写入。
    pub fn writes_power(&self) -> bool {
        !self.writes.is_empty()
    }
}

/// 包住真正的设置存储：电源设置的写入先记在这里（发布前不会写下去），读取依次看这里记下的、队里还没写的、
/// 预读的；其余照常。
pub(crate) struct WriteBehind<'a> {
    inner: &'a dyn SettingStore,
    queue: &'a PowerQueue,
    prefetched: &'a Prefetched,
    staged: RefCell<Vec<(Target, Value)>>,
    activate: Cell<bool>,
}

impl<'a> WriteBehind<'a> {
    pub fn new(
        inner: &'a dyn SettingStore,
        queue: &'a PowerQueue,
        prefetched: &'a Prefetched,
    ) -> Self {
        Self {
            inner,
            queue,
            prefetched,
            staged: RefCell::new(Vec::new()),
            activate: Cell::new(false),
        }
    }

    /// 记下的写入，交给 `PowerWriter::commit` 发布。
    pub fn into_staged(self) -> Staged {
        Staged {
            writes: self.staged.into_inner(),
            activate: self.activate.get(),
        }
    }
}

impl SettingStore for WriteBehind<'_> {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError> {
        if !self.staged.borrow().is_empty() || !self.queue.is_empty() {
            // 只用预读到的当前计划解析（不在调度线程上问电源服务）；没有预读时按原样查。
            let place = queued_place(target, |t| {
                self.prefetched.resolve(t).ok_or(StoreError::Unsupported)
            });
            if let Some(v) = latest(&self.staged.borrow(), &place) {
                return Ok(Some(v));
            }
            if let Some(v) = self.queue.latest(&place) {
                return Ok(Some(v));
            }
        }
        match self.prefetched.value(target) {
            Some(v) => Ok(v),
            None => self.inner.read(target),
        }
    }

    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError> {
        let Target::PowerSetting { .. } = target else {
            return self.inner.write(target, value);
        };
        // 执行线程只写具体的计划：没写明的先换成当前计划（变更日志交来的已经换好了）。
        // 换不了时直接写（存储自己会报错）。
        match self.resolve(target) {
            Ok(
                place @ Target::PowerSetting {
                    scheme: Some(_), ..
                },
            ) => {
                self.staged.borrow_mut().push((place, value.clone()));
                Ok(())
            }
            _ => self.inner.write(target, value),
        }
    }

    fn delete(&self, target: &Target) -> Result<(), StoreError> {
        self.inner.delete(target)
    }

    fn resolve(&self, target: &Target) -> Result<Target, StoreError> {
        match self.prefetched.resolve(target) {
            Some(t) => Ok(t),
            None => self.inner.resolve(target),
        }
    }

    fn begin_batch(&self) {
        self.inner.begin_batch();
    }

    fn end_batch(&self) {
        if self.inner.end_batch_deferred() {
            self.activate.set(true);
        }
    }

    /// 和 [`WriteBehind::end_batch`] 相同：要重新激活的随发布交给执行线程，调用方不用再管。
    fn end_batch_deferred(&self) -> bool {
        self.end_batch();
        false
    }
}

/// 发布记下的写入：排进队列（预读的这些位置作废）。返回是否需要通知执行线程写入、重新激活。
pub(crate) fn publish(queue: &PowerQueue, prefetched: &Prefetched, staged: Staged) -> (bool, bool) {
    let write = !staged.writes.is_empty();
    for (place, _) in &staged.writes {
        prefetched.forget(place);
    }
    queue.extend(staged.writes);
    (write, staged.activate)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use poise_core::journal::Journal;

    use super::super::queue::PendingWrites;
    use super::*;

    #[derive(Default)]
    struct Mem(RefCell<HashMap<Target, Value>>);
    impl SettingStore for Mem {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            Ok(self.0.borrow().get(t).cloned())
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.0.borrow_mut().insert(t.clone(), v.clone());
            Ok(())
        }
        fn delete(&self, t: &Target) -> Result<(), StoreError> {
            self.0.borrow_mut().remove(t);
            Ok(())
        }
    }

    fn power(scheme: Option<&str>) -> Target {
        Target::PowerSetting {
            scheme: scheme.map(Into::into),
            subgroup: "S".into(),
            setting: "X".into(),
            ac: true,
        }
    }

    /// 调度引擎的一次修改：经 [`WriteBehind`] 修改，之后发布。
    fn edit<T>(
        store: &dyn SettingStore,
        queue: &PowerQueue,
        prefetched: &Prefetched,
        f: impl FnOnce(&dyn SettingStore) -> T,
    ) -> T {
        let behind = WriteBehind::new(store, queue, prefetched);
        let out = f(&behind);
        let _ = publish(queue, prefetched, behind.into_staged());
        out
    }

    /// 执行线程写完队里的全部写入。
    fn flush(queue: &PowerQueue, store: &Mem) {
        let writes = queue.pending();
        for (t, v) in &writes {
            store.write(t, v).unwrap();
        }
        queue.done(writes.len(), Vec::new());
    }

    #[test]
    fn power_writes_wait_for_the_commit_and_everything_else_goes_through() {
        let store = Mem::default();
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        let behind = WriteBehind::new(&store, &queue, &prefetched);
        let registry = Target::Registry {
            hive: poise_core::setting::Hive::LocalMachine,
            key: "K".into(),
            name: "N".into(),
        };
        behind.write(&power(Some("A")), &Value::Dword(1)).unwrap();
        behind.write(&registry, &Value::Dword(2)).unwrap();
        behind.write(&power(Some("A")), &Value::Dword(3)).unwrap();
        assert_eq!(
            behind.read(&power(Some("a"))).unwrap(),
            Some(Value::Dword(3))
        );
        // 发布之前执行线程看不到（变更日志还没落盘）。
        assert!(queue.is_empty());
        assert_eq!(store.read(&power(Some("A"))).unwrap(), None);
        assert_eq!(store.read(&registry).unwrap(), Some(Value::Dword(2)));
        let staged = behind.into_staged();
        assert!(staged.writes_power());
        assert_eq!(publish(&queue, &prefetched, staged), (true, false));
        assert_eq!(
            queue.pending(),
            [
                (power(Some("A")), Value::Dword(1)),
                (power(Some("A")), Value::Dword(3))
            ]
        );
    }

    /// 当前计划是 A 的存储：优化项的位置不写明计划，按当前计划解析。
    struct OnPlanA(Mem);
    impl SettingStore for OnPlanA {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            self.0.read(t)
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.0.write(t, v)
        }
        fn delete(&self, t: &Target) -> Result<(), StoreError> {
            self.0.delete(t)
        }
        fn resolve(&self, t: &Target) -> Result<Target, StoreError> {
            Ok(match t {
                Target::PowerSetting { scheme: None, .. } => power(Some("A")),
                other => other.clone(),
            })
        }
    }

    #[test]
    fn places_without_a_plan_see_what_is_queued_for_the_current_plan() {
        let store = OnPlanA(Mem::default());
        let queue = PowerQueue::default();
        queue.extend(vec![(power(Some("A")), Value::Dword(100))]);
        // 电源守护、优化项列表读优化项（不写明计划）：以队里当前计划的值为准，不读到还没写的旧值。
        let pending = PendingWrites::new(queue.clone());
        assert_eq!(
            pending.view(&store).read(&power(None)).unwrap(),
            Some(Value::Dword(100))
        );
        // 调度线程上（WriteBehind）只用预读到的当前计划解析，不为此去问电源服务：没有预读时按原样读。
        let prefetched = Prefetched::default();
        let behind = WriteBehind::new(&store, &queue, &prefetched);
        assert_eq!(behind.read(&power(None)).unwrap(), None);
    }

    #[test]
    fn until_written_the_queued_value_is_the_current_value() {
        let store = Mem::default();
        store.write(&power(Some("A")), &Value::Dword(50)).unwrap();
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        // 接管（写 100），又撤销（写回原值 50），都还没写进去：此刻的值是 50，不是 100。
        edit(&store, &queue, &prefetched, |s| {
            s.write(&power(Some("A")), &Value::Dword(100)).unwrap();
            s.write(&power(Some("A")), &Value::Dword(50)).unwrap();
        });
        let pending = PendingWrites::new(queue.clone());
        let view = pending.view(&store);
        store.write(&power(Some("A")), &Value::Dword(7)).unwrap();
        assert_eq!(
            view.read(&power(Some("a"))).unwrap(),
            Some(Value::Dword(50))
        );
        // 执行线程写完第一项：第二项还在队里，仍以它为准；都写完后读真正的存储。
        queue.done(1, Vec::new());
        assert_eq!(
            view.read(&power(Some("A"))).unwrap(),
            Some(Value::Dword(50))
        );
        queue.done(1, Vec::new());
        assert!(queue.is_empty());
        assert_eq!(view.read(&power(Some("A"))).unwrap(), Some(Value::Dword(7)));
    }

    #[test]
    fn retaking_a_setting_before_its_undo_is_written_keeps_the_users_original() {
        let store = Mem::default();
        let t = power(Some("A"));
        store.write(&t, &Value::Dword(50)).unwrap();
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        let mut j = Journal::default();
        edit(&store, &queue, &prefetched, |s| {
            j.apply(s, "mode", &t, &Value::Dword(100), 0).unwrap();
        });
        flush(&queue, &store);
        // 换到不管这个设置的模式（撤销排进队里还没写），马上又换回来。
        edit(&store, &queue, &prefetched, |s| {
            j.revert(s, "mode").unwrap()
        });
        edit(&store, &queue, &prefetched, |s| {
            j.apply(s, "mode", &t, &Value::Dword(100), 0).unwrap();
        });
        // 同一次修改里撤销又接管（还没发布）也一样。
        edit(&store, &queue, &prefetched, |s| {
            j.revert(s, "mode").unwrap();
            j.apply(s, "mode", &t, &Value::Dword(100), 0).unwrap();
        });
        flush(&queue, &store);
        assert_eq!(store.read(&t).unwrap(), Some(Value::Dword(100)));
        // Poise 退出：回到用户原来的 50，而不是 Poise 自己写过的 100。
        j.revert(&store, "mode").unwrap();
        assert_eq!(store.read(&t).unwrap(), Some(Value::Dword(50)));
    }

    #[test]
    fn unresolvable_power_places_are_written_at_once() {
        let store = Mem::default();
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        let behind = WriteBehind::new(&store, &queue, &prefetched);
        behind.write(&power(None), &Value::Dword(1)).unwrap();
        assert!(!behind.into_staged().writes_power());
        assert_eq!(store.read(&power(None)).unwrap(), Some(Value::Dword(1)));
    }

    /// 结束一批时总要重新激活电源计划的存储。
    struct Activating;
    impl SettingStore for Activating {
        fn read(&self, _: &Target) -> Result<Option<Value>, StoreError> {
            Ok(None)
        }
        fn write(&self, _: &Target, _: &Value) -> Result<(), StoreError> {
            Ok(())
        }
        fn delete(&self, _: &Target) -> Result<(), StoreError> {
            Ok(())
        }
        fn end_batch_deferred(&self) -> bool {
            true
        }
    }

    #[test]
    fn reactivating_the_plan_goes_with_the_commit() {
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        let behind = WriteBehind::new(&Activating, &queue, &prefetched);
        behind.begin_batch();
        assert!(!behind.end_batch_deferred());
        let staged = behind.into_staged();
        assert!(!staged.writes_power());
        assert_eq!(publish(&queue, &prefetched, staged), (false, true));
    }

    #[test]
    fn prefetched_values_and_plan_are_used_until_the_place_is_written() {
        let store = Mem::default();
        store.write(&power(Some("A")), &Value::Dword(7)).unwrap();
        let queue = PowerQueue::default();
        let prefetched = Prefetched::default();
        let generation = prefetched.ask().unwrap();
        // 预读时当前计划是 A，值是 7（之后不再问存储）。
        prefetched.fill(&Resolving(&store), &[power(None)], generation);
        store.write(&power(Some("A")), &Value::Dword(9)).unwrap();
        let behind = WriteBehind::new(&Unreachable, &queue, &prefetched);
        assert_eq!(behind.resolve(&power(None)).unwrap(), power(Some("A")));
        assert_eq!(
            behind.read(&power(Some("A"))).unwrap(),
            Some(Value::Dword(7))
        );
        // 没写明计划的也按预读的计划排队；发布后预读的值作废。
        behind.write(&power(None), &Value::Dword(1)).unwrap();
        let _ = publish(&queue, &prefetched, behind.into_staged());
        assert_eq!(queue.pending(), [(power(Some("A")), Value::Dword(1))]);
        queue.done(1, Vec::new());
        assert!(prefetched.value(&power(Some("A"))).is_none());
    }

    /// 把没写明计划的电源设置换成计划 A。
    struct Resolving<'a>(&'a Mem);
    impl SettingStore for Resolving<'_> {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            self.0.read(t)
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.0.write(t, v)
        }
        fn delete(&self, t: &Target) -> Result<(), StoreError> {
            self.0.delete(t)
        }
        fn resolve(&self, t: &Target) -> Result<Target, StoreError> {
            Ok(match t {
                Target::PowerSetting { scheme: None, .. } => power(Some("A")),
                _ => t.clone(),
            })
        }
    }

    /// 一调用就失败：证明没有去问真正的存储。
    struct Unreachable;
    impl SettingStore for Unreachable {
        fn read(&self, _: &Target) -> Result<Option<Value>, StoreError> {
            Err(StoreError::Unsupported)
        }
        fn write(&self, _: &Target, _: &Value) -> Result<(), StoreError> {
            Err(StoreError::Unsupported)
        }
        fn delete(&self, _: &Target) -> Result<(), StoreError> {
            Err(StoreError::Unsupported)
        }
        fn resolve(&self, _: &Target) -> Result<Target, StoreError> {
            Err(StoreError::Unsupported)
        }
    }
}
