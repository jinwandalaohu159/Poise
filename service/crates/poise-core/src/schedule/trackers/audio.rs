//! 发声保持。

use std::collections::HashMap;

use crate::classify::Classification;
use crate::model::{Millis, Pid};

/// 进程树最近一次发声的时间。停声后保持一会儿仍算"在发声"：切歌、暂停、语音里的短暂安静
/// 不会让它来回切换状态。
#[derive(Debug, Default)]
pub struct AudioTracker {
    heard_at: HashMap<Pid, Millis>,
}

impl AudioTracker {
    pub fn update(&mut self, classes: &HashMap<Pid, Classification>, now: Millis, hold_ms: Millis) {
        for c in classes.values().filter(|c| c.plays_audio) {
            self.heard_at.insert(c.root_pid, now);
        }
        self.heard_at
            .retain(|_, t| now.saturating_sub(*t) < hold_ms);
    }

    /// 这个进程树是否在发声，或刚停声不久。
    pub fn audible(&self, root: Pid) -> bool {
        self.heard_at.contains_key(&root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::trackers::test_classes;

    #[test]
    fn short_pauses_keep_the_program_audible() {
        let mut t = AudioTracker::default();
        let mut playing = test_classes(None);
        playing.get_mut(&10).unwrap().plays_audio = true;
        let silent = test_classes(None);
        t.update(&playing, 0, 30_000);
        t.update(&silent, 20_000, 30_000);
        assert!(t.audible(10));
        assert!(!t.audible(20));
        t.update(&silent, 31_000, 30_000);
        assert!(!t.audible(10));
    }
}
