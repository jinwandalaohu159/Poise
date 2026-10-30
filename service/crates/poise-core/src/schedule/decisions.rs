//! 程序级的调度决定及其变化。只在变化时报告，供日志记录：事后能看出每个程序什么时候、
//! 为什么被调整，以及当时有多少人在等处理器。

use std::collections::HashMap;

use super::trackers::Pressure;
use crate::action::CoreSet;
use crate::model::ProcessKey;

/// 对一个程序（进程树）的决定，按它的主进程算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 不调整，或只是保护（不受系统节能限速）。
    Normal,
    /// 按游戏调度。
    Playing,
    /// 重点程序：正被直接使用、持续需要处理器，优先用大核。
    Focused,
    /// 后台忙，完整让路（让开主域、节能、降优先级）。
    Yielded,
    /// 后台忙，只降优先级（正在用的程序需要时优先）。
    Lowered,
    /// 后台忙、但不能降优先级的程序（发声、自己设为高优先级），有重点程序时移到小核。
    Backseat,
    /// 有重点程序时，没在用的程序移到小核。数量多、变化频繁，不逐个记录（见 [`Decision::Aside`]）。
    Aside,
}

/// 程序不（再）让路的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// 在前台。
    Foreground,
    /// 程序自己把优先级调到了"高"（录屏、串流等）。
    SelfElevated,
    /// 用户设为低优先级（让路的原因）。
    MarkedLow,
    /// 刚用过（离开前台不久）。
    RecentlyUsed,
    /// 正在发声。
    Audio,
    /// 临时全速。
    Boosted,
    /// 用户设为"始终优先"或"游戏"。
    Marked,
    /// 持续一段时间没人需要处理器。
    NoDemand,
    /// 需求降低，它已远低于新的门槛。
    LessDemand,
    /// 程序退出了。
    Exited,
}

/// 一个程序此刻的决定、核心范围与负载（单核百分比、磁盘读写 MB/s），以及不让路时的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramState {
    pub verdict: Verdict,
    pub cores: CoreSet,
    pub load_pct: u32,
    pub disk_mb_s: u32,
    pub cause: Option<Cause>,
}

/// 一次变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 对处理器的需求变了。
    Pressure(Pressure),
    /// 对一个程序的决定变了，或重点程序占用的核心变了（`state` 是变化后的）。
    Program {
        root: ProcessKey,
        from: Verdict,
        state: ProgramState,
    },
    /// 移到小核、给重点程序让出大核的程序：开始时报告有几个，全部移回时报告 0。
    Aside(usize),
}

/// 记着上一轮的决定，每轮报告变化。退出的程序直接忘掉；只有游戏退出时报告一次。
#[derive(Debug, Default)]
pub struct DecisionLog {
    pressure: Pressure,
    programs: HashMap<ProcessKey, (Verdict, CoreSet)>,
    aside: usize,
}

impl DecisionLog {
    pub fn update(
        &mut self,
        pressure: Pressure,
        programs: impl IntoIterator<Item = (ProcessKey, ProgramState)>,
    ) -> Vec<Decision> {
        let mut changes = Vec::new();
        if pressure != self.pressure {
            self.pressure = pressure;
            changes.push(Decision::Pressure(pressure));
        }
        let mut current = HashMap::with_capacity(self.programs.len());
        let mut aside = 0;
        for (root, state) in programs {
            let (from, cores) = self
                .programs
                .remove(&root)
                .unwrap_or((Verdict::Normal, CoreSet::All));
            aside += usize::from(state.verdict == Verdict::Aside);
            if reportable(from, cores, &state) {
                changes.push(Decision::Program { root, from, state });
            }
            current.insert(root, (state.verdict, state.cores));
        }
        // 剩下的是这一轮不在了的程序。
        for (root, (from, cores)) in std::mem::replace(&mut self.programs, current) {
            if from == Verdict::Playing {
                changes.push(Decision::Program {
                    root,
                    from,
                    state: ProgramState {
                        verdict: Verdict::Normal,
                        cores,
                        load_pct: 0,
                        disk_mb_s: 0,
                        cause: Some(Cause::Exited),
                    },
                });
            }
        }
        if (aside == 0) != (self.aside == 0) {
            changes.push(Decision::Aside(aside));
        }
        self.aside = aside;
        changes
    }
}

/// 要不要记录这次变化：决定变了（移到小核、从小核移回不逐个记录），或者重点程序占用的核心变了。
fn reportable(from: Verdict, cores: CoreSet, state: &ProgramState) -> bool {
    let quiet = |v| matches!(v, Verdict::Normal | Verdict::Aside);
    if from != state.verdict {
        return !(quiet(from) && quiet(state.verdict));
    }
    matches!(from, Verdict::Playing | Verdict::Focused) && cores != state.cores
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey { pid: 7, created: 1 };

    fn state(verdict: Verdict, load_pct: u32) -> ProgramState {
        ProgramState {
            verdict,
            cores: CoreSet::All,
            load_pct,
            disk_mb_s: 0,
            cause: None,
        }
    }

    #[test]
    fn only_changes_are_reported() {
        let mut log = DecisionLog::default();
        assert!(
            log.update(Pressure::None, [(KEY, state(Verdict::Normal, 0))])
                .is_empty()
        );
        let changes = log.update(Pressure::Normal, [(KEY, state(Verdict::Yielded, 40))]);
        assert_eq!(
            changes,
            vec![
                Decision::Pressure(Pressure::Normal),
                Decision::Program {
                    root: KEY,
                    from: Verdict::Normal,
                    state: state(Verdict::Yielded, 40),
                },
            ]
        );
        assert!(
            log.update(Pressure::Normal, [(KEY, state(Verdict::Yielded, 30))])
                .is_empty()
        );
        // 退出的程序不报告。
        assert!(log.update(Pressure::Normal, []).is_empty());
    }

    #[test]
    fn games_that_exit_are_reported_once() {
        let mut log = DecisionLog::default();
        log.update(Pressure::High, [(KEY, state(Verdict::Playing, 300))]);
        let changes = log.update(Pressure::High, []);
        assert!(matches!(
            changes.as_slice(),
            [Decision::Program { from: Verdict::Playing, state, .. }] if state.cause == Some(Cause::Exited)
        ));
        assert!(log.update(Pressure::High, []).is_empty());
    }

    #[test]
    fn programs_moved_aside_are_summarized_and_core_changes_of_focused_ones_reported() {
        let mut log = DecisionLog::default();
        let other = ProcessKey { pid: 8, created: 1 };
        let changes = log.update(
            Pressure::High,
            [
                (KEY, state(Verdict::Focused, 300)),
                (other, state(Verdict::Aside, 0)),
            ],
        );
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[2], Decision::Aside(1));
        let wider = ProgramState {
            cores: CoreSet::Domains(0b11),
            ..state(Verdict::Focused, 1000)
        };
        let changes = log.update(
            Pressure::High,
            [(KEY, wider), (other, state(Verdict::Aside, 0))],
        );
        assert!(
            matches!(changes.as_slice(), [Decision::Program { state, .. }] if state.cores == wider.cores)
        );
        let changes = log.update(Pressure::High, [(other, state(Verdict::Normal, 0))]);
        assert_eq!(changes, vec![Decision::Aside(0)]);
    }
}
