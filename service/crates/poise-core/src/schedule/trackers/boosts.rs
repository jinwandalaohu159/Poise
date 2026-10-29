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

    pub fn expire(&mut self, now: Millis) {
        self.until.retain(|_, t| *t > now);
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
        b.expire(1000);
        assert!(b.roots(&classes).is_empty());
    }
}
