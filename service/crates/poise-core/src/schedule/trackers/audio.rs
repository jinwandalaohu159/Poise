//! 发声保持。

use std::collections::HashMap;

use crate::classify::Classification;
use crate::model::{Millis, Pid, ProcessKey, Snapshot};

/// 最近一次发声的时间：按程序（进程树）记，也按真正输出声音的进程记。停声后保持一会儿仍算
/// "在发声"：切歌、暂停、语音里的短暂安静不会让它来回切换状态。
#[derive(Debug, Default)]
pub struct AudioTracker {
    trees: HashMap<Pid, Millis>,
    emitters: HashMap<ProcessKey, Millis>,
}

impl AudioTracker {
    pub fn update(
        &mut self,
        snap: &Snapshot,
        classes: &HashMap<Pid, Classification>,
        now: Millis,
        hold_ms: Millis,
    ) {
        for p in &snap.processes {
            let Some(c) = classes.get(&p.facts.key.pid) else {
                continue;
            };
            if c.plays_audio {
                self.trees.insert(c.root_pid, now);
            }
            if c.emits_audio {
                self.emitters.insert(p.facts.key, now);
            }
        }
        self.trees.retain(|_, t| now.saturating_sub(*t) < hold_ms);
        self.emitters
            .retain(|_, t| now.saturating_sub(*t) < hold_ms);
    }

    /// 这个程序（进程树）是否在发声，或刚停声不久。
    pub fn audible(&self, root: Pid) -> bool {
        self.trees.contains_key(&root)
    }

    /// 这个进程是否在输出声音，或刚停下不久。
    pub fn emitting(&self, key: &ProcessKey) -> bool {
        self.emitters.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcessFacts, ProcessSample};
    use crate::schedule::trackers::test_classes;

    #[test]
    fn short_pauses_keep_the_program_audible() {
        let snap = Snapshot {
            processes: [10, 20]
                .map(|pid| ProcessSample {
                    facts: ProcessFacts {
                        key: ProcessKey { pid, created: 1 },
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .to_vec(),
            ..Default::default()
        };
        let key = ProcessKey {
            pid: 10,
            created: 1,
        };
        let mut t = AudioTracker::default();
        let mut playing = test_classes(None);
        let c = playing.get_mut(&10).unwrap();
        c.plays_audio = true;
        c.emits_audio = true;
        let silent = test_classes(None);
        t.update(&snap, &playing, 0, 30_000);
        t.update(&snap, &silent, 20_000, 30_000);
        assert!(t.audible(10) && t.emitting(&key));
        assert!(!t.audible(20));
        t.update(&snap, &silent, 31_000, 30_000);
        assert!(!t.audible(10) && !t.emitting(&key));
    }
}
