//! 临时全速。

use std::collections::HashMap;

use crate::model::{Millis, Pid, ProcessKey};

/// 用户在托盘里选择的"临时全速"，到期自动失效。作用于整个进程树。
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

    pub fn is_boosted(&self, key: &ProcessKey, root: Pid) -> bool {
        self.until.keys().any(|k| k == key || k.pid == root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boosts_expire_and_cover_the_tree() {
        let mut b = TempBoosts::default();
        b.add(
            ProcessKey {
                pid: 10,
                created: 1,
            },
            1000,
        );
        assert!(b.is_boosted(
            &ProcessKey {
                pid: 11,
                created: 2
            },
            10
        ));
        b.expire(1000);
        assert!(!b.is_boosted(
            &ProcessKey {
                pid: 10,
                created: 1
            },
            10
        ));
    }
}
