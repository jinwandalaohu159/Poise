//! 程序级的调度决定及其变化。只在变化时报告，供日志记录：事后能看出每个程序什么时候、
//! 为什么被调整，以及当时有多少人在等处理器。

use std::collections::HashMap;

use super::policy::Desired;
use super::trackers::Pressure;
use crate::model::ProcessKey;

/// 对一个程序（进程树）的决定，按它的主进程算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 不调整，或只是保护（不受系统节能限速）。
    Normal,
    /// 按游戏调度。
    Playing,
    /// 后台忙，给别人让路。
    Yielded,
    /// 玩游戏时后台忙的发声程序，移到小核。
    Backseat,
}

impl Verdict {
    pub fn of(desired: Desired) -> Self {
        if desired == Desired::PLAYING || desired == Desired::GAME {
            Self::Playing
        } else if desired == Desired::BACKSEAT {
            Self::Backseat
        } else if desired == Desired::YIELDED {
            Self::Yielded
        } else {
            Self::Normal
        }
    }
}

/// 程序不（再）让路的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// 在前台。
    Foreground,
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

/// 一个程序此刻的决定与负载（单核百分比、磁盘读写 MB/s），以及不让路时的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramState {
    pub verdict: Verdict,
    pub load_pct: u32,
    pub disk_mb_s: u32,
    pub cause: Option<Cause>,
}

/// 一次变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 对处理器的需求变了。
    Pressure(Pressure),
    /// 对一个程序的决定变了（`state` 是变化后的）。
    Program {
        root: ProcessKey,
        from: Verdict,
        state: ProgramState,
    },
}

/// 记着上一轮的决定，每轮报告变化。退出的程序直接忘掉；只有游戏退出时报告一次。
#[derive(Debug, Default)]
pub struct DecisionLog {
    pressure: Pressure,
    programs: HashMap<ProcessKey, Verdict>,
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
        for (root, state) in programs {
            let from = self.programs.remove(&root).unwrap_or(Verdict::Normal);
            if from != state.verdict {
                changes.push(Decision::Program { root, from, state });
            }
            current.insert(root, state.verdict);
        }
        // 剩下的是这一轮不在了的程序。
        for (root, from) in std::mem::replace(&mut self.programs, current) {
            if from == Verdict::Playing {
                changes.push(Decision::Program {
                    root,
                    from,
                    state: ProgramState {
                        verdict: Verdict::Normal,
                        load_pct: 0,
                        disk_mb_s: 0,
                        cause: Some(Cause::Exited),
                    },
                });
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey { pid: 7, created: 1 };

    fn state(verdict: Verdict, load_pct: u32) -> ProgramState {
        ProgramState {
            verdict,
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
}
