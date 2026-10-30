//! 核心与缓存面板：处理器的核心域（见 `CpuTopology`）、各自的缓存与此刻的负载，以及调度把哪些程序放在
//! 哪个域上（见 `Scheduler::programs`）；没有限制核心的程序里负载最高的几个另外列出（由 Windows 安排）。
//! 只读、只用已有的结果（不额外采样），界面显示面板时每 2 秒取一次。
//!
//! Windows 不提供"哪个程序占了多少缓存"（那需要服务器处理器的缓存监控功能），这里按核心说话：
//! 限制在某个核心域上的程序只用那个域的缓存；同一个域里的程序共用它的二级缓存，共用三级缓存的域见 `l3_group`。

use poise_core::action::CoreSet;
use poise_core::model::{ProcessKey, Snapshot};
use poise_core::schedule::{ProgramState, Verdict};
use poise_platform::observe::topology::CoreDomain;
use poise_protocol::{CoreDomainDto, CoreMap, PlacedProgram, ProcessorDto};

use super::Engine;

/// 由 Windows 安排的程序最多列几个。
const MAX_UNRESTRICTED: usize = 8;
/// 负载（单核百分比）至少这么多才列出来（闲着的程序列出来没有意义）。
const MIN_UNRESTRICTED_LOAD: u32 = 3;

impl Engine {
    pub(super) fn core_map(&self) -> CoreMap {
        let loads = self.activity.processor_loads();
        let mut domains: Vec<CoreDomainDto> = self
            .topology
            .domains
            .iter()
            .map(|d| domain(d, loads))
            .collect();
        let mut unrestricted = Vec::new();
        if let Some(view) = &self.view {
            unrestricted = place_programs(&mut domains, self.scheduler.programs(), &view.snapshot);
        }
        CoreMap {
            partitioned: self.topology.partitioned(),
            smart_schedule: self.settings.smart_schedule,
            demand: self.scheduler.pressure(),
            domains,
            unrestricted,
        }
    }
}

/// 一个核心域的描述与负载（程序另外放进去）。`loads` 是第 0 个处理器组各逻辑处理器的占用，还没有读数时为空。
fn domain(d: &CoreDomain, loads: &[f32]) -> CoreDomainDto {
    let processors: Vec<ProcessorDto> = d
        .processors
        .iter()
        .map(|p| ProcessorDto {
            number: u32::from(p.group) * 64 + u32::from(p.index),
            core: u32::from(p.group) * 256 + u32::from(p.core),
            // 只读得到第 0 组的负载（见 `poise_platform::observe::processors`）。
            load_pct: (p.group == 0)
                .then(|| loads.get(usize::from(p.index)).copied())
                .flatten(),
        })
        .collect();
    let measured: Vec<f32> = processors.iter().filter_map(|p| p.load_pct).collect();
    CoreDomainDto {
        performance: d.performance,
        cores: d.cores,
        threads: d.ids.len() as u32,
        l2_bytes: d.l2_bytes,
        l2_threads: d.l2_threads,
        l3_bytes: d.llc_bytes,
        l3_group: d.llc_block.map(u32::from),
        load_pct: (!measured.is_empty())
            .then(|| measured.iter().sum::<f32>() / measured.len() as f32),
        processors,
        programs: Vec::new(),
    }
}

/// 把限制了核心的程序放进它用到的每个域；返回没有限制核心、负载最高的几个程序。
fn place_programs(
    domains: &mut [CoreDomainDto],
    programs: &[(ProcessKey, ProgramState)],
    snapshot: &Snapshot,
) -> Vec<PlacedProgram> {
    let count = domains.len();
    let every = CoreSet::All.domain_mask(count);
    let mut unrestricted = Vec::new();
    for (key, state) in programs {
        let mask = state.cores.domain_mask(count);
        let free = mask == every;
        if free && state.load_pct < MIN_UNRESTRICTED_LOAD {
            continue;
        }
        let Some(p) = snapshot.find(key.pid).filter(|p| p.facts.key == *key) else {
            continue;
        };
        let placed = PlacedProgram {
            pid: key.pid,
            name: p.facts.friendly_name(),
            path: p.facts.image_path.clone().unwrap_or_default(),
            verdict: state.verdict,
            load_pct: state.load_pct,
            domains: mask.count_ones(),
        };
        if free {
            unrestricted.push(placed);
            continue;
        }
        for (i, d) in domains.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                d.programs.push(placed.clone());
            }
        }
    }
    for d in domains {
        d.programs
            .sort_by_key(|p| (rank(p.verdict), std::cmp::Reverse(p.load_pct)));
    }
    unrestricted.sort_by_key(|p| std::cmp::Reverse(p.load_pct));
    unrestricted.truncate(MAX_UNRESTRICTED);
    unrestricted
}

/// 列表里的先后：重点程序，让路的，挪开的。
fn rank(v: Verdict) -> u8 {
    match v {
        Verdict::Playing => 0,
        Verdict::Focused => 1,
        Verdict::Yielded => 2,
        Verdict::Lowered => 3,
        Verdict::Backseat => 4,
        Verdict::Aside => 5,
        Verdict::Normal => 6,
    }
}

#[cfg(test)]
mod tests {
    use poise_platform::observe::topology::LogicalProcessor;

    use super::*;

    fn d(processors: &[(u16, u8, u8)]) -> CoreDomain {
        CoreDomain {
            ids: processors.iter().map(|&(_, i, _)| u32::from(i)).collect(),
            processors: processors
                .iter()
                .map(|&(group, index, core)| LogicalProcessor { group, index, core })
                .collect(),
            cores: 2,
            performance: false,
            l2_bytes: 2 << 20,
            l2_threads: 4,
            llc_bytes: 24 << 20,
            llc_block: Some(0),
        }
    }

    #[test]
    fn domains_report_cores_and_the_load_that_can_be_read() {
        let loads = [10.0, 30.0, 50.0];
        let first = domain(&d(&[(0, 0, 0), (0, 1, 0)]), &loads);
        assert_eq!(first.load_pct, Some(20.0));
        assert_eq!(first.l3_group, Some(0));
        // 超线程的两个逻辑处理器同属一个核心。
        assert_eq!(first.processors[0].core, first.processors[1].core);
        let second = domain(&d(&[(0, 2, 1), (1, 0, 0)]), &loads);
        // 第 1 组的处理器读不到负载：编号接着第 0 组往后排，平均只算读得到的。
        assert_eq!(second.processors[1].number, 64);
        assert_ne!(second.processors[1].core, first.processors[0].core);
        assert_eq!(second.processors[1].load_pct, None);
        assert_eq!(second.load_pct, Some(50.0));
    }
}
