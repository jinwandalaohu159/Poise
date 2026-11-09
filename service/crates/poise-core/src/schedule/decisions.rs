//! 程序级的调度决定及其变化。只在变化时报告，供日志记录：事后能看出每个程序什么时候、
//! 为什么被调整，以及当时有多少人在等处理器。

use crate::fast_hash::HashMap;
use serde::{Deserialize, Serialize};

use super::trackers::{DemandReason, Pressure};
use crate::action::CoreSet;
use crate::model::{Millis, ProcessKey};

/// 对一个程序（进程树）的决定，按它的主进程算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// 用户的程序（用过的、由它们启动的、正在为它们干活的）在后台干活：不当成后台让路、不节能，
    /// 只在有竞争时分处理器（见 `policy::WorkRoom`）。
    Working,
    /// 后台忙、但不能降优先级的程序（发声、自己设为高优先级），有重点程序时移到小核。
    Backseat,
    /// 有重点程序时，没在用的程序移到小核。数量多、变化频繁，不逐个记录（见 [`Decision::Aside`]）。
    Aside,
}

/// 程序不（再）让路的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// 像游戏（前台全屏、显卡在渲染），还在等待确认：先放到大核。
    LooksLikeGame,
    /// 重点程序的负载降下来了。
    LoadDropped,
    /// 重点程序离开前台太久。
    LeftForeground,
    /// 在玩游戏：后台的工作让开游戏的核心并节能，功耗留给游戏（见 `policy::WorkRoom::Yielded`）。
    Gaming,
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
    /// 对处理器的需求变了，以及从哪来。
    Pressure(Pressure, Option<DemandReason>),
    /// 对一个程序的决定变了，或重点程序占用的核心变了（`state` 是变化后的）。
    Program {
        root: ProcessKey,
        from: Verdict,
        state: ProgramState,
    },
    /// 移到小核、给重点程序让出大核的程序：开始时报告有几个，全部移回时报告 0。
    Aside(usize),
}

/// 记着上一轮的决定，每轮报告变化。游戏退出时报告一次；别的程序不在了不报告。
///
/// 一个程序这一轮不在，不一定是退出了：进程树会变（辅助进程有时归到主程序名下，有时自己成为一个程序），
/// 它过一会儿可能又出现。不在的程序的决定再记一会儿（见 [`DecisionLog::FORGET_AFTER_MS`]），重新出现时
/// 决定没变就不报告——实测有个游戏的辅助进程这样时隐时现，整局游戏每 4 秒报告一次"用大核"。
#[derive(Debug, Default)]
pub struct DecisionLog {
    pressure: Pressure,
    programs: HashMap<ProcessKey, Seen>,
    /// 这一轮之前不在了的程序：最后的决定，以及从什么时候起不在。
    gone: HashMap<ProcessKey, (Seen, Millis)>,
    aside: usize,
}

/// 上一轮报告时一个程序的决定（和这一轮比较）。
#[derive(Debug, Clone, Copy)]
struct Seen {
    verdict: Verdict,
    cores: CoreSet,
    cause: Option<Cause>,
}

impl Seen {
    const NORMAL: Self = Self {
        verdict: Verdict::Normal,
        cores: CoreSet::All,
        cause: None,
    };

    fn of(state: &ProgramState) -> Self {
        Self {
            verdict: state.verdict,
            cores: state.cores,
            cause: state.cause,
        }
    }
}

impl DecisionLog {
    /// 不在了的程序的决定记多久。
    const FORGET_AFTER_MS: Millis = 60_000;

    pub fn update(
        &mut self,
        pressure: Pressure,
        reason: Option<DemandReason>,
        programs: impl IntoIterator<Item = (ProcessKey, ProgramState)>,
        now: Millis,
    ) -> Vec<Decision> {
        let mut changes = Vec::new();
        if pressure != self.pressure {
            self.pressure = pressure;
            changes.push(Decision::Pressure(pressure, reason));
        }
        self.gone
            .retain(|_, (_, since)| now.saturating_sub(*since) < Self::FORGET_AFTER_MS);
        let mut current =
            HashMap::with_capacity_and_hasher(self.programs.len(), Default::default());
        let mut aside = 0;
        for (root, state) in programs {
            let seen = self
                .programs
                .remove(&root)
                .or_else(|| self.gone.remove(&root).map(|(seen, _)| seen))
                .unwrap_or(Seen::NORMAL);
            aside += usize::from(state.verdict == Verdict::Aside);
            if reportable(seen, &state) {
                changes.push(Decision::Program {
                    root,
                    from: seen.verdict,
                    state,
                });
            }
            current.insert(root, Seen::of(&state));
        }
        // 剩下的是这一轮不在了的程序。
        for (root, seen) in std::mem::replace(&mut self.programs, current) {
            let Seen {
                verdict: from,
                cores,
                ..
            } = seen;
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
            } else {
                self.gone.insert(root, (seen, now));
            }
        }
        if (aside == 0) != (self.aside == 0) {
            changes.push(Decision::Aside(aside));
        }
        self.aside = aside;
        changes
    }
}

/// 要不要记录这次变化：决定变了（移到小核、从小核移回不逐个记录），重点程序占用的核心变了，或者在干活的
/// 程序换了一种用法（不限、和重点程序切开、移开，是否为游戏让路）——留几个核心的增减不逐次记录。
fn reportable(seen: Seen, state: &ProgramState) -> bool {
    let quiet = |v| matches!(v, Verdict::Normal | Verdict::Aside);
    let from = seen.verdict;
    if from != state.verdict {
        return !(quiet(from) && quiet(state.verdict));
    }
    match from {
        Verdict::Playing | Verdict::Focused => seen.cores != state.cores,
        Verdict::Working => {
            let gaming = |c: Option<Cause>| c == Some(Cause::Gaming);
            work_kind(seen.cores) != work_kind(state.cores)
                || gaming(seen.cause) != gaming(state.cause)
        }
        _ => false,
    }
}

/// 在干活的程序用核心的方式：不限、和重点程序切开、移开（分到哪组小核不算变化）。
fn work_kind(cores: CoreSet) -> u8 {
    match cores {
        CoreSet::All => 0,
        CoreSet::Rest(_) => 1,
        c if c.is_efficiency() => 2,
        _ => 3,
    }
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
            log.update(Pressure::None, None, [(KEY, state(Verdict::Normal, 0))], 0)
                .is_empty()
        );
        let changes = log.update(
            Pressure::Normal,
            None,
            [(KEY, state(Verdict::Yielded, 40))],
            0,
        );
        assert_eq!(
            changes,
            vec![
                Decision::Pressure(Pressure::Normal, None),
                Decision::Program {
                    root: KEY,
                    from: Verdict::Normal,
                    state: state(Verdict::Yielded, 40),
                },
            ]
        );
        assert!(
            log.update(
                Pressure::Normal,
                None,
                [(KEY, state(Verdict::Yielded, 30))],
                0
            )
            .is_empty()
        );
        // 退出的程序不报告。
        assert!(log.update(Pressure::Normal, None, [], 0).is_empty());
    }

    #[test]
    fn games_that_exit_are_reported_once() {
        let mut log = DecisionLog::default();
        log.update(
            Pressure::High,
            None,
            [(KEY, state(Verdict::Playing, 300))],
            0,
        );
        let changes = log.update(Pressure::High, None, [], 0);
        assert!(matches!(
            changes.as_slice(),
            [Decision::Program { from: Verdict::Playing, state, .. }] if state.cause == Some(Cause::Exited)
        ));
        assert!(log.update(Pressure::High, None, [], 0).is_empty());
    }

    #[test]
    fn programs_moved_aside_are_summarized_and_core_changes_of_focused_ones_reported() {
        let mut log = DecisionLog::default();
        let other = ProcessKey { pid: 8, created: 1 };
        let changes = log.update(
            Pressure::High,
            None,
            [
                (KEY, state(Verdict::Focused, 300)),
                (other, state(Verdict::Aside, 0)),
            ],
            0,
        );
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[2], Decision::Aside(1));
        let wider = ProgramState {
            cores: CoreSet::Domains(0b11),
            ..state(Verdict::Focused, 1000)
        };
        let changes = log.update(
            Pressure::High,
            None,
            [(KEY, wider), (other, state(Verdict::Aside, 0))],
            0,
        );
        assert!(
            matches!(changes.as_slice(), [Decision::Program { state, .. }] if state.cores == wider.cores)
        );
        let changes = log.update(
            Pressure::High,
            None,
            [(other, state(Verdict::Normal, 0))],
            0,
        );
        assert_eq!(changes, vec![Decision::Aside(0)]);
    }

    #[test]
    fn work_changing_how_it_uses_the_cores_is_reported_but_not_every_share_step() {
        let mut log = DecisionLog::default();
        let work = |cores, cause| ProgramState {
            cores,
            cause,
            ..state(Verdict::Working, 800)
        };
        let reported = |log: &mut DecisionLog, s: ProgramState, now| {
            !log.update(Pressure::Normal, None, [(KEY, s)], now)
                .is_empty()
        };
        assert!(reported(&mut log, work(CoreSet::All, None), 0));
        // 和重点程序切开：记一次；留几个核心的增减不逐次记。
        assert!(reported(&mut log, work(CoreSet::Rest(4), None), 2_000));
        assert!(!reported(&mut log, work(CoreSet::Rest(5), None), 4_000));
        // 开了一局游戏：判定还是"在干活"，换成为游戏让路要记下来；分到哪组小核不算变化。
        let gaming = Some(Cause::Gaming);
        assert!(reported(
            &mut log,
            work(CoreSet::Domains(0b010), gaming),
            6_000
        ));
        assert!(!reported(
            &mut log,
            work(CoreSet::Domains(0b100), gaming),
            8_000
        ));
        // 游戏结束，回到不限核心：记一次。
        assert!(reported(&mut log, work(CoreSet::All, None), 10_000));
    }

    #[test]
    fn programs_that_briefly_vanish_are_not_reported_again() {
        let mut log = DecisionLog::default();
        let focused = state(Verdict::Focused, 0);
        assert_eq!(
            log.update(Pressure::High, None, [(KEY, focused)], 0).len(),
            2
        );
        // 时隐时现：同样的决定不再报告。
        for t in 1..10 {
            let now = t * 2_000;
            let present = if t % 2 == 0 {
                vec![(KEY, focused)]
            } else {
                vec![]
            };
            assert!(
                log.update(Pressure::High, None, present, now).is_empty(),
                "{now}"
            );
        }
        // 很久以后再出现：当作新的程序。
        log.update(Pressure::High, None, [], 30_000);
        assert_eq!(
            log.update(Pressure::High, None, [(KEY, focused)], 100_000)
                .len(),
            1
        );
    }
}
