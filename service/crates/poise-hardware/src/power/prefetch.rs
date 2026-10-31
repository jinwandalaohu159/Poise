//! 预先读好的电源设置：第一次接管一个模式的电源设置时（服务启动、Poise 重新打开、换了电源计划）要先读出
//! 每项的原值（撤销时写回），还要问一次当前是哪个电源计划。电源服务忙的时候（正是登录、换计划的时候）
//! 每读一项要几十到几百毫秒，一个模式二三十项，实测在调度线程里一卡就是好几秒。
//!
//! 改成在电源执行线程里预先读好（见 `actuator`），调度引擎下一轮直接用：
//!
//! - 读到的值只用于紧接着的那一次接管，用完就丢（之后别处可能改了它）；排进写入队列的位置立即作废。
//! - 当前电源计划一直沿用，直到换了计划或还原（电源守护发现换了计划时通知，见 `Hardware::power_plan_changed`），
//!   之后每次切换模式都不用再问电源服务。

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use poise_core::setting::{SettingStore, Target, Value};

#[derive(Default)]
enum State {
    /// 没有预读的结果。
    #[default]
    Empty,
    /// 已经交给执行线程，还没读完。
    Asked {
        /// 这一次预读的代次：清空过之后，旧的一次读完交回来的结果不算数。
        generation: u64,
        since: Instant,
        /// 读的期间排进了写入的位置：读到的可能是写入之前的值，不用。
        dirty: Vec<Target>,
        /// 等太久已经记过日志。
        warned: bool,
    },
    /// 读完了：当前电源计划（读不到时为空）与各位置此刻的值（位置都已换成具体的电源计划）。
    Ready {
        scheme: Option<String>,
        values: Vec<(Target, Option<Value>)>,
    },
}

#[derive(Default)]
struct Inner {
    state: State,
    generation: u64,
}

/// 预读的结果，调度引擎和硬件执行线程共用。
#[derive(Clone, Default)]
pub(crate) struct Prefetched(Arc<Mutex<Inner>>);

impl Prefetched {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 还没预读过：交给执行线程，返回这一次的代次（之后返回 None，直到清空）。
    pub fn ask(&self) -> Option<u64> {
        let mut inner = self.lock();
        if !matches!(inner.state, State::Empty) {
            return None;
        }
        let generation = inner.generation;
        inner.state = State::Asked {
            generation,
            since: Instant::now(),
            dirty: Vec::new(),
            warned: false,
        };
        Some(generation)
    }

    pub fn ready(&self) -> bool {
        matches!(self.lock().state, State::Ready { .. })
    }

    /// 交给执行线程超过 `after` 还没读完（电源服务卡住了）：只在第一次超过时为真。
    pub fn newly_overdue(&self, after: Duration) -> bool {
        match &mut self.lock().state {
            State::Asked { since, warned, .. } if !*warned && since.elapsed() >= after => {
                *warned = true;
                true
            }
            _ => false,
        }
    }

    /// 执行线程：读出各位置此刻的值（`store` 读到的就是此刻的值：排着的写入已经写完）。
    /// 只有 `generation` 仍是正在等的那一次时才收下。
    pub fn fill(&self, store: &dyn SettingStore, places: &[Target], generation: u64) {
        store.begin_batch();
        let mut values: Vec<(Target, Option<Value>)> = places
            .iter()
            .filter_map(|t| store.resolve(t).ok())
            .filter_map(|t| store.read(&t).ok().map(|v| (t, v)))
            .collect();
        store.end_batch_deferred();
        let scheme = values.iter().find_map(|(t, _)| match t {
            Target::PowerSetting {
                scheme: Some(s), ..
            } => Some(s.clone()),
            _ => None,
        });
        let mut inner = self.lock();
        // 读的期间清空过（换了计划、还原了）：结果作废。
        if let State::Asked {
            generation: asked,
            dirty,
            ..
        } = &inner.state
            && *asked == generation
        {
            values.retain(|(t, _)| !dirty.iter().any(|d| d.same_place(t)));
            inner.state = State::Ready { scheme, values };
        }
    }

    /// 没写明计划的电源设置补上预读到的当前计划；没有预读结果时为空。
    pub fn resolve(&self, target: &Target) -> Option<Target> {
        let Target::PowerSetting {
            scheme: None,
            subgroup,
            setting,
            ac,
        } = target
        else {
            return None;
        };
        match &self.lock().state {
            State::Ready {
                scheme: Some(scheme),
                ..
            } => Some(Target::PowerSetting {
                scheme: Some(scheme.clone()),
                subgroup: subgroup.clone(),
                setting: setting.clone(),
                ac: *ac,
            }),
            _ => None,
        }
    }

    /// 预读到的这个位置的值；没有读过这个位置时为 None。
    pub fn value(&self, target: &Target) -> Option<Option<Value>> {
        match &self.lock().state {
            State::Ready { values, .. } => values
                .iter()
                .find(|(t, _)| t.same_place(target))
                .map(|(_, v)| v.clone()),
            _ => None,
        }
    }

    /// 这个位置要写入新值了：预读的值作废（还在读的，读到后不用）。
    pub fn forget(&self, target: &Target) {
        match &mut self.lock().state {
            State::Ready { values, .. } => values.retain(|(t, _)| !t.same_place(target)),
            // 电源服务卡住、一直读不完时也不越记越多：同一位置只记一次。
            State::Asked { dirty, .. } => {
                if !dirty.iter().any(|d| d.same_place(target)) {
                    dirty.push(target.clone());
                }
            }
            State::Empty => {}
        }
    }

    /// 接管完了：读到的值都作废，当前计划留着。
    pub fn forget_values(&self) {
        if let State::Ready { values, .. } = &mut self.lock().state {
            values.clear();
        }
    }

    /// 换了电源计划、还原了：全部作废，还在读的那一次读完也不算数。
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.state = State::Empty;
        inner.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use poise_core::setting::StoreError;

    use super::*;

    /// 当前计划是 "A"；没写过的设置读到 50。
    #[derive(Default)]
    struct Mem(RefCell<HashMap<Target, Value>>);
    impl SettingStore for Mem {
        fn read(&self, t: &Target) -> Result<Option<Value>, StoreError> {
            Ok(Some(
                self.0.borrow().get(t).cloned().unwrap_or(Value::Dword(50)),
            ))
        }
        fn write(&self, t: &Target, v: &Value) -> Result<(), StoreError> {
            self.0.borrow_mut().insert(t.clone(), v.clone());
            Ok(())
        }
        fn delete(&self, _: &Target) -> Result<(), StoreError> {
            Err(StoreError::Unsupported)
        }
        fn resolve(&self, t: &Target) -> Result<Target, StoreError> {
            Ok(match t {
                Target::PowerSetting {
                    scheme: None,
                    subgroup,
                    setting,
                    ac,
                } => Target::PowerSetting {
                    scheme: Some("A".into()),
                    subgroup: subgroup.clone(),
                    setting: setting.clone(),
                    ac: *ac,
                },
                _ => t.clone(),
            })
        }
    }

    fn power(scheme: Option<&str>, setting: &str) -> Target {
        Target::PowerSetting {
            scheme: scheme.map(Into::into),
            subgroup: "S".into(),
            setting: setting.into(),
            ac: true,
        }
    }

    #[test]
    fn values_are_used_once_and_the_plan_until_cleared() {
        let p = Prefetched::default();
        let generation = p.ask().unwrap();
        assert_eq!(p.ask(), None);
        assert!(!p.ready());
        p.fill(
            &Mem::default(),
            &[power(None, "X"), power(None, "Y")],
            generation,
        );
        assert!(p.ready());
        assert_eq!(p.resolve(&power(None, "Z")), Some(power(Some("A"), "Z")));
        assert_eq!(
            p.value(&power(Some("a"), "X")),
            Some(Some(Value::Dword(50)))
        );
        p.forget(&power(Some("A"), "X"));
        assert_eq!(p.value(&power(Some("A"), "X")), None);
        p.forget_values();
        assert_eq!(p.value(&power(Some("A"), "Y")), None);
        assert_eq!(p.resolve(&power(None, "Z")), Some(power(Some("A"), "Z")));
        p.clear();
        assert_eq!(p.resolve(&power(None, "Z")), None);
        assert!(p.ask().is_some());
    }

    #[test]
    fn results_read_after_clearing_are_dropped() {
        let p = Prefetched::default();
        let old = p.ask().unwrap();
        p.clear();
        p.fill(&Mem::default(), &[power(None, "X")], old);
        assert!(!p.ready());
        // 清空后马上又问了一次：旧的一次读完交回来的也不算，只收新的。
        let new = p.ask().unwrap();
        p.fill(&Mem::default(), &[power(None, "X")], old);
        assert!(!p.ready());
        p.fill(&Mem::default(), &[power(None, "X")], new);
        assert!(p.ready());
    }

    #[test]
    fn places_written_while_reading_are_not_used() {
        let p = Prefetched::default();
        let generation = p.ask().unwrap();
        p.forget(&power(Some("A"), "X"));
        p.fill(
            &Mem::default(),
            &[power(None, "X"), power(None, "Y")],
            generation,
        );
        assert_eq!(p.value(&power(Some("A"), "X")), None);
        assert_eq!(
            p.value(&power(Some("A"), "Y")),
            Some(Some(Value::Dword(50)))
        );
    }

    #[test]
    fn waiting_too_long_is_reported_once() {
        let p = Prefetched::default();
        assert!(!p.newly_overdue(Duration::ZERO));
        p.ask();
        assert!(p.newly_overdue(Duration::ZERO));
        assert!(!p.newly_overdue(Duration::ZERO));
        assert!(!Prefetched::default().newly_overdue(Duration::from_secs(60)));
    }
}
