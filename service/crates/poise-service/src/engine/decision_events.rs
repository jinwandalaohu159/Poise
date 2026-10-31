//! 把调度决定的变化（见 `poise_core::schedule::Decision`）写成日志事件：补上程序的名称和路径，
//! 把核心范围换成"用大核和几组小核"这样的说法。决定本身见 `poise_core::schedule`，事件的记录见 `poise_logbook`。

use poise_core::action::CoreSet;
use poise_core::model::{ProcessKey, Snapshot};
use poise_core::schedule::Decision;
use poise_protocol::{CoreSpan, Event, ProgramRef};

/// 这一轮决定的变化对应的事件。`previous` 是上一轮的进程快照（刚退出的程序在那里找名称），
/// `domains` 是处理器的核心域个数。
pub(super) fn events(
    decisions: &[Decision],
    snapshot: &Snapshot,
    previous: Option<&Snapshot>,
    domains: usize,
) -> Vec<Event> {
    decisions
        .iter()
        .map(|d| match d {
            Decision::Pressure(level, reason) => Event::Demand {
                level: *level,
                reason: *reason,
            },
            Decision::Aside(n) => Event::MovedAside { count: *n as u32 },
            Decision::Program { root, from, state } => Event::Program {
                program: program_ref(*root, std::iter::once(snapshot).chain(previous)),
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

/// 程序的名称和路径：依次在给出的快照里找（这一轮已经没有的——刚退出的——到上一轮的里找）；
/// 都找不到时只剩进程号。
pub(super) fn program_ref<'a>(
    key: ProcessKey,
    snapshots: impl IntoIterator<Item = &'a Snapshot>,
) -> ProgramRef {
    let sample = snapshots
        .into_iter()
        .find_map(|s| s.find(key.pid).filter(|p| p.facts.key == key));
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
    use poise_core::model::{ProcessFacts, ProcessSample};

    use super::*;

    #[test]
    fn programs_that_just_exited_are_named_from_the_previous_round() {
        let key = ProcessKey {
            pid: 19212,
            created: 5,
        };
        let game = ProcessSample {
            facts: ProcessFacts {
                key,
                name: "league of legends.exe".into(),
                display_name: Some("League of Legends".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let previous = Snapshot {
            processes: vec![game],
            ..Default::default()
        };
        let now = Snapshot::default();
        assert_eq!(
            program_ref(key, std::iter::once(&now).chain(Some(&previous))).name,
            "League of Legends"
        );
        assert_eq!(program_ref(key, [&now]).name, "19212");
    }

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
