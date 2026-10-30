//! 把进程表、静态属性缓存、友好名称、服务表、系统指标（含处理器实际性能）组装成 poise-core 的 [`Snapshot`]。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use poise_core::model::{Millis, ProcessFacts, ProcessKey, ProcessSample, Snapshot};

use super::enumerate::{ProcessTable, RawProcess};
use super::facts::FactsCache;
use super::names::NameResolver;
use crate::observe::cpu_performance::CpuPerformanceMeter;
use crate::observe::services::ServiceMap;
use crate::observe::system;
use crate::persist::paths;

#[derive(Default)]
pub struct Snapshotter {
    table: ProcessTable,
    facts: FactsCache,
    names: NameResolver,
    services: ServiceMap,
    cpu: CpuPerformanceMeter,
}

/// 采集一次超过这么久时，日志里写明每一步用了多久（多半是在等系统服务回应，而不是在算）。
const SLOW_TAKE: Duration = Duration::from_millis(100);

impl Snapshotter {
    pub fn take(&mut self, at: Millis) -> windows::core::Result<Snapshot> {
        let mut steps = Steps::start();
        let raw = self.table.read()?;
        steps.done("进程表");
        self.services.refresh(service_hosts(&raw));
        steps.done("服务");
        self.names.collect();
        steps.done("名称");
        let alive: HashSet<ProcessKey> = raw.iter().map(|r| r.key).collect();
        self.facts.retain(|k| alive.contains(k));

        let processes = raw
            .into_iter()
            .map(|r| {
                let s = self.facts.get(r.key);
                let display_name = s.image_path.as_deref().and_then(|p| self.names.name(p));
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
                        display_name,
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
        steps.done("进程信息");
        let mut system = system::read_metrics();
        steps.done("整机指标");
        if let Some(p) = self.cpu.sample() {
            system.cpu_mhz_current =
                (system.cpu_mhz_base as f32 * p.performance_pct / 100.0).round() as u32;
            system.cpu_perf_limit_pct = Some(p.limit_pct);
        }
        steps.done("处理器性能");
        steps.log_if_slow();
        Ok(Snapshot {
            at,
            processes,
            system,
        })
    }
}

/// 采集的各步用时。
struct Steps {
    started: Instant,
    last: Instant,
    done: Vec<(&'static str, Duration)>,
}

impl Steps {
    fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            done: Vec::new(),
        }
    }

    fn done(&mut self, step: &'static str) {
        let now = Instant::now();
        self.done.push((step, now - self.last));
        self.last = now;
    }

    fn log_if_slow(&self) {
        let total = self.started.elapsed();
        if total < SLOW_TAKE {
            return;
        }
        let parts: Vec<String> = self
            .done
            .iter()
            .map(|(step, d)| format!("{step} {}", d.as_millis()))
            .collect();
        log::info!(
            "采集进程用了 {} ms（毫秒）：{}",
            total.as_millis(),
            parts.join("，")
        );
    }
}

/// 服务管理器（services.exe）拉起的进程，也就是各个服务的宿主进程；找不到服务管理器时为 None。
fn service_hosts(raw: &[RawProcess]) -> Option<HashSet<ProcessKey>> {
    let scm = raw
        .iter()
        .find(|r| r.session_id == 0 && r.name == "services.exe")?
        .key
        .pid;
    Some(
        raw.iter()
            .filter(|r| r.parent_pid == scm)
            .map(|r| r.key)
            .collect(),
    )
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
        // 服务宿主进程：服务管理器拉起的进程里有承载服务的。
        let raw = s.table.read().unwrap();
        let hosts = super::service_hosts(&raw).expect("找不到服务管理器");
        assert!(
            snap.processes
                .iter()
                .any(|p| hosts.contains(&p.facts.key) && !p.facts.services.is_empty())
        );
    }
}
