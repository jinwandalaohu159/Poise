//! 把调度决定的变化（见 `poise_core::schedule::Decision`）写成日志事件：补上程序的名称和路径，
//! 把核心范围换成"用大核和几组小核"这样的说法。决定本身见 `poise_core::schedule`，事件的记录见 `crate::events`。

use poise_core::action::CoreSet;
use poise_core::model::{ProcessKey, Snapshot};
use poise_core::schedule::Decision;
use poise_protocol::{CoreSpan, Event, ProgramRef};

/// 这一轮决定的变化对应的事件。`domains` 是处理器的核心域个数。
pub(super) fn events(decisions: &[Decision], snapshot: &Snapshot, domains: usize) -> Vec<Event> {
    decisions
        .iter()
        .map(|d| match d {
            Decision::Pressure(level) => Event::Demand { level: *level },
            Decision::Aside(n) => Event::MovedAside { count: *n as u32 },
            Decision::Program { root, from, state } => Event::Program {
                program: program_ref(*root, snapshot),
                from: *from,
                to: state.verdict,
                cores: core_span(state.cores, domains),
                load_pct: state.load_pct,
                disk_mb_s: state.disk_mb_s,
                cause: state.cause,
            },
        })
        .collect()
}

/// 程序的名称和路径；已经退出的只剩进程号。
pub(super) fn program_ref(key: ProcessKey, snapshot: &Snapshot) -> ProgramRef {
    let sample = snapshot.find(key.pid).filter(|p| p.facts.key == key);
    ProgramRef {
        pid: key.pid,
        name: sample.map_or_else(|| key.pid.to_string(), |p| p.facts.friendly_name()),
        path: sample
            .and_then(|p| p.facts.image_path.clone())
            .unwrap_or_default(),
    }
}

/// 核心范围的说法：第 0 个核心域是主域（大核），其余是按缓存分的组。
pub(super) fn core_span(cores: CoreSet, domains: usize) -> CoreSpan {
    let mask = cores.domain_mask(domains);
    let all = CoreSet::All.domain_mask(domains);
    CoreSpan {
        all: mask == all,
        main: mask & 1 != 0,
        others: (mask & !1).count_ones(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_ranges_read_as_big_cores_plus_groups() {
        let span = |set| core_span(set, 3);
        assert!(span(CoreSet::All).all);
        assert_eq!(
            span(CoreSet::Domains(0b011)),
            CoreSpan {
                all: false,
                main: true,
                others: 1
            }
        );
        assert!(!span(CoreSet::Efficiency).main);
        // 只有一个域的处理器：怎样都是所有核心。
        assert!(core_span(CoreSet::Performance, 1).all);
    }
}
