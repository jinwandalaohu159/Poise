//! 临时全速。

use std::collections::{HashMap, HashSet};

use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey};

/// 用户在托盘里选择的"临时全速"，到期自动失效。作用于所选进程所在的整个程序（进程树）。
#[derive(Debug, Default)]
pub struct TempBoosts {
    until: HashMap<ProcessKey, Millis>,
}

impl TempBoosts {
    pub fn add(&mut self, key: ProcessKey, until: Millis) {
        self.until.insert(key, until);
    }

    /// 到期的、进程已经退出的都去掉：进程号可能被新进程复用，不能让它继承"临时全速"。
    pub fn retain(&mut self, alive: &HashSet<ProcessKey>, now: Millis) {
        self.until.retain(|k, t| *t > now && alive.contains(k));
    }

    /// 被临时全速的程序（进程树的根）。
    pub fn roots(&self, classes: &HashMap<Pid, Classification>) -> HashSet<Pid> {
        self.until
            .keys()
            .filter_map(|k| classes.get(&k.pid))
            .map(|c| c.root_pid)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::trackers::test_classes;

    #[test]
    fn boosts_expire_and_cover_the_program() {
        let mut b = TempBoosts::default();
        b.add(
            ProcessKey {
                pid: 10,
                created: 1,
            },
            1000,
        );
        let classes = test_classes(None);
        assert_eq!(b.roots(&classes), HashSet::from([10]));
        let alive = HashSet::from([ProcessKey {
            pid: 10,
            created: 1,
        }]);
        b.retain(&alive, 999);
        assert_eq!(b.roots(&classes).len(), 1);
        b.retain(&alive, 1000);
        assert!(b.roots(&classes).is_empty());
    }

    #[test]
    fn a_boost_ends_when_its_process_exits() {
        let mut b = TempBoosts::default();
        b.add(
            ProcessKey {
                pid: 10,
                created: 1,
            },
            1000,
        );
        // 同一个进程号，换了一个进程。
        b.retain(
            &HashSet::from([ProcessKey {
                pid: 10,
                created: 2,
            }]),
            0,
        );
        assert!(b.roots(&test_classes(None)).is_empty());
    }
}
