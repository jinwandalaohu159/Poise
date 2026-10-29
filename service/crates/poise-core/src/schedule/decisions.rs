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

/// 一次变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 对处理器的需求变了。
    Pressure(Pressure),
    /// 对一个程序的决定变了。`load_pct` 是它当时平滑后的负载（单核百分比）。
    Program {
        root: ProcessKey,
        from: Verdict,
        to: Verdict,
        load_pct: u32,
    },
}

/// 记着上一轮的决定，每轮报告变化。退出的程序直接忘掉，不报告。
#[derive(Debug, Default)]
pub struct DecisionLog {
    pressure: Pressure,
    programs: HashMap<ProcessKey, Verdict>,
}

impl DecisionLog {
    pub fn update(
        &mut self,
        pressure: Pressure,
        programs: impl IntoIterator<Item = (ProcessKey, Verdict, u32)>,
    ) -> Vec<Decision> {
        let mut changes = Vec::new();
        if pressure != self.pressure {
            self.pressure = pressure;
            changes.push(Decision::Pressure(pressure));
        }
        let mut current = HashMap::with_capacity(self.programs.len());
        for (root, to, load_pct) in programs {
            let from = self.programs.get(&root).copied().unwrap_or(Verdict::Normal);
            if from != to {
                changes.push(Decision::Program {
                    root,
                    from,
                    to,
                    load_pct,
                });
            }
            current.insert(root, to);
        }
        self.programs = current;
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: ProcessKey = ProcessKey { pid: 7, created: 1 };

    #[test]
    fn only_changes_are_reported() {
        let mut log = DecisionLog::default();
        assert!(
            log.update(Pressure::None, [(KEY, Verdict::Normal, 0)])
                .is_empty()
        );
        let changes = log.update(Pressure::Normal, [(KEY, Verdict::Yielded, 40)]);
        assert_eq!(
            changes,
            vec![
                Decision::Pressure(Pressure::Normal),
                Decision::Program {
                    root: KEY,
                    from: Verdict::Normal,
                    to: Verdict::Yielded,
                    load_pct: 40,
                },
            ]
        );
        assert!(
            log.update(Pressure::Normal, [(KEY, Verdict::Yielded, 30)])
                .is_empty()
        );
        // 退出的程序不报告。
        assert!(log.update(Pressure::Normal, []).is_empty());
    }
}
