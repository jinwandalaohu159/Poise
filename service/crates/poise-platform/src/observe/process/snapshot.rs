//! 把进程表、静态属性缓存、服务表、系统指标组装成 poise-core 的 [`Snapshot`]。

use std::collections::HashSet;

use poise_core::model::{Millis, ProcessFacts, ProcessKey, ProcessSample, Snapshot};

use super::enumerate::ProcessTable;
use super::facts::FactsCache;
use crate::observe::services::ServiceMap;
use crate::observe::system;
use crate::persist::paths;

#[derive(Default)]
pub struct Snapshotter {
    table: ProcessTable,
    facts: FactsCache,
    services: ServiceMap,
}

impl Snapshotter {
    pub fn take(&mut self, at: Millis) -> windows::core::Result<Snapshot> {
        let raw = self.table.read()?;
        self.services.refresh_if_stale();
        let alive: HashSet<ProcessKey> = raw.iter().map(|r| r.key).collect();
        self.facts.retain(|k| alive.contains(k));

        let processes = raw
            .into_iter()
            .map(|r| {
                let s = self.facts.get(r.key);
                ProcessSample {
                    facts: ProcessFacts {
                        key: r.key,
                        parent_pid: (r.parent_pid != 0).then_some(r.parent_pid),
                        name: r.name,
                        in_windows_dir: s
                            .image_path
                            .as_deref()
                            .is_some_and(paths::is_in_windows_dir),
                        image_path: s.image_path,
                        display_name: s.display_name,
                        session_id: r.session_id,
                        is_critical: s.is_critical,
                        protection: s.protection,
                        priority: Some(r.base_priority),
                        services: self.services.services_of(r.key.pid),
                    },
                    metrics: r.metrics,
                }
            })
            .collect();
        Ok(Snapshot {
            at,
            processes,
            system: system::read_metrics(),
        })
    }
}

#[cfg(test)]
mod tests {
    use poise_core::model::Protection;

    #[test]
    fn snapshot_contains_self_with_facts() {
        let mut s = super::Snapshotter::default();
        let snap = s.take(1).unwrap();
        let me = snap.find(std::process::id()).expect("self");
        assert!(me.facts.image_path.is_some());
        assert_eq!(me.facts.protection, Protection::None);
        assert!(snap.system.mem_total > 0);
        // 第二次采样走缓存，不应出错。
        assert!(s.take(2).is_ok());
    }
}
