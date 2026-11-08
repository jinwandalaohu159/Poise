//! 程序归谁：是用户的，还是后台自己的。对每个程序都按同一条规则判断，不看它是什么程序。
//!
//! 在后台占用处理器的程序不一定是"后台程序"。用户在编辑器里点了编译、在终端里跑起脚本，然后切去看网页
//! ——这些工作正是用户在等的。Windows 却会把没有前台窗口的程序当成后台（系统自动节能），放到小核、
//! 压低频率。所以先分清归属：
//!
//! - 用户的：被用户直接用过（在前台过、临时全速、设为"始终优先"），或者是被这样的程序启动的（第一次看到
//!   它时，它的父进程属于这样的程序；桌面外壳不算——从桌面打开的程序要自己被用过才算）；以及正在为
//!   用户的程序干活的（经本机网络连着，见 `working::serving_roots`）。
//! - 其余是后台自己的（更新、索引、同步、云盘、开机自己启动的托盘程序）。
//!
//! 用户的程序在忙（和后台"在忙"同一个门槛，见 `BusyRule`），就是"在干活"：不当成后台让路、不节能，
//! 只在有竞争时分处理器（见 `policy::WorkRoom`）。降到门槛的一半以下一段时间才不算，编译的链接、
//! 脚本的单线程阶段不会因此掉出去。

use super::load::TreeUsage;
use crate::fast_hash::{HashMap, HashSet};
use crate::model::{Millis, ProcessKey};

/// 在干活的程序什么时候不再算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkRule {
    /// 负载降到"在忙"门槛的这个比例以下……
    pub leave_ratio: f32,
    /// ……持续这么久，不再算在干活。
    pub leave_after_ms: Millis,
}

/// 第一次看到一个程序时查清的：谁启动了它。
#[derive(Debug, Clone, Copy, Default)]
pub struct Sighting {
    /// 启动它的程序（它的主进程的父进程所属的程序）。
    pub parent: Option<ProcessKey>,
}

/// 一个程序此刻的情况。
#[derive(Debug, Clone, Copy)]
pub struct WorkSignals {
    /// 能不能算在干活：正在玩的游戏、临时全速、用户设过类别的照它们自己的规则。
    pub eligible: bool,
    /// 正在为用户的程序干活（经本机网络连着）。
    pub serving: bool,
    /// 平滑后的负载（慢的：开始算看它，短暂的尖峰不算）。
    pub usage: TreeUsage,
    /// 快的平滑处理器负载（单核百分比）：停下来看它，干完了一分钟内就不再算。
    pub recent: f32,
    /// "在忙"的门槛：处理器（单核百分比）、磁盘（字节每秒）。
    pub busy: (f32, f64),
}

#[derive(Debug, Default)]
pub struct WorkTracker {
    /// 被用户直接用过的程序，或由它们启动的（见模块说明）。
    engaged: HashSet<ProcessKey>,
    /// 已经看到过的程序（启动它的程序只在第一次看到时查）。
    seen: HashSet<ProcessKey>,
    /// 在干活的程序，以及负载降到退出门槛以下的时刻。
    working: HashMap<ProcessKey, Option<Millis>>,
    /// `engaged` 变过几次（服务据此决定要不要重新保存，见 [`WorkTracker::revision`]）。
    revision: u64,
}

impl WorkTracker {
    /// 这个程序已经看到过（[`Sighting`] 只在第一次看到时查）。
    pub fn knows(&self, root: &ProcessKey) -> bool {
        self.seen.contains(root)
    }

    /// 每轮对每个程序调用一次：`used` 表示它此刻被用户直接用着；第一次看到它时带上 `sighting`
    /// （之后的忽略）。
    pub fn note(&mut self, root: ProcessKey, used: bool, sighting: Option<Sighting>) {
        let inherited = self.seen.insert(root)
            && sighting
                .and_then(|s| s.parent)
                .is_some_and(|p| self.engaged.contains(&p));
        if (inherited || used) && self.engaged.insert(root) {
            self.revision += 1;
        }
    }

    /// 被用户直接用过的程序变过几次：没变时不必重新取 [`WorkTracker::engaged_keys`]。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 被用户直接用过，或由这样的程序启动。
    pub fn engaged(&self, root: &ProcessKey) -> bool {
        self.engaged.contains(root)
    }

    /// 全部被用户直接用过的程序（排好序，服务把它存下来）。
    pub fn engaged_keys(&self) -> Vec<ProcessKey> {
        let mut keys: Vec<ProcessKey> = self.engaged.iter().copied().collect();
        keys.sort_by_key(|k| (k.pid, k.created));
        keys
    }

    /// 接上之前记下的（服务重启、暂停后继续）。已经退出的下一轮就忘掉。
    pub fn restore(&mut self, keys: &[ProcessKey]) {
        self.engaged.extend(keys.iter().copied());
        self.revision += 1;
    }

    /// 每轮对每个程序调用一次（在 [`WorkTracker::note`] 之后），返回它是不是在干活。
    pub fn observe(
        &mut self,
        root: ProcessKey,
        signals: WorkSignals,
        rule: WorkRule,
        now: Millis,
    ) -> bool {
        let owned = signals.serving || self.engaged(&root);
        if !signals.eligible || !owned {
            self.working.remove(&root);
            return false;
        }
        let (core, disk) = signals.busy;
        let disk_busy = |ratio: f32| signals.usage.disk_bytes_per_s >= disk * f64::from(ratio);
        // 开始：持续够久（慢的）而且此刻也在忙（快的）——刚干完时慢的平滑值还要好一阵才降下来。
        let starts = (signals.usage.core_pct >= core && signals.recent >= core) || disk_busy(1.0);
        let keeps = signals.recent >= core * rule.leave_ratio || disk_busy(rule.leave_ratio);
        match self.working.get_mut(&root) {
            Some(light_since) if keeps => {
                *light_since = None;
                true
            }
            Some(light_since) => {
                let since = *light_since.get_or_insert(now);
                let still = now.saturating_sub(since) < rule.leave_after_ms;
                if !still {
                    self.working.remove(&root);
                }
                still
            }
            None if starts => {
                self.working.insert(root, None);
                true
            }
            None => false,
        }
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        let before = self.engaged.len();
        self.engaged.retain(|k| alive.contains(k));
        if self.engaged.len() != before {
            self.revision += 1;
        }
        self.seen.retain(|k| alive.contains(k));
        self.working.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: WorkRule = WorkRule {
        leave_ratio: 0.5,
        leave_after_ms: 30_000,
    };
    const TERMINAL: ProcessKey = ProcessKey { pid: 1, created: 1 };
    const PYTHON: ProcessKey = ProcessKey { pid: 2, created: 2 };
    const UPDATER: ProcessKey = ProcessKey { pid: 3, created: 3 };
    const MB: f64 = 1024.0 * 1024.0;

    fn load(core_pct: f32) -> WorkSignals {
        WorkSignals {
            eligible: true,
            serving: false,
            usage: TreeUsage {
                core_pct,
                disk_bytes_per_s: 0.0,
            },
            recent: core_pct,
            busy: (10.0, 8.0 * MB),
        }
    }

    #[test]
    fn busy_programs_of_the_user_are_working_until_they_calm_down() {
        let mut w = WorkTracker::default();
        w.note(TERMINAL, true, None);
        assert!(!w.observe(TERMINAL, load(5.0), RULE, 0));
        // 训练脚本、编译、写数据：处理器或磁盘达到"在忙"的门槛。
        assert!(w.observe(TERMINAL, load(45.0), RULE, 2_000));
        let writing = WorkSignals {
            usage: TreeUsage {
                core_pct: 2.0,
                disk_bytes_per_s: 50.0 * MB,
            },
            ..load(0.0)
        };
        assert!(w.observe(TERMINAL, writing, RULE, 4_000));
        // 单线程的阶段、短暂的停顿：还在干活。
        assert!(w.observe(TERMINAL, load(6.0), RULE, 6_000));
        assert!(w.observe(TERMINAL, load(1.0), RULE, 8_000));
        assert!(!w.observe(TERMINAL, load(1.0), RULE, 40_000));
    }

    #[test]
    fn programs_started_by_engaged_ones_inherit_it_but_only_at_first_sight() {
        let mut w = WorkTracker::default();
        let by_terminal = Some(Sighting {
            parent: Some(TERMINAL),
        });
        w.note(TERMINAL, true, None);
        w.note(PYTHON, false, by_terminal);
        assert!(w.engaged(&PYTHON));
        // 自己在后台启动、从没被用过的程序不算；之后也不会因为别的程序被用而"继承"。
        w.note(UPDATER, false, None);
        assert!(w.knows(&UPDATER));
        w.note(UPDATER, false, by_terminal);
        assert!(!w.engaged(&UPDATER));
        assert!(!w.observe(UPDATER, load(400.0), RULE, 0));
    }

    #[test]
    fn programs_serving_the_user_count_without_being_used() {
        let mut w = WorkTracker::default();
        w.note(UPDATER, false, None);
        let serving = WorkSignals {
            serving: true,
            ..load(30.0)
        };
        assert!(w.observe(UPDATER, serving, RULE, 0));
        // 游戏、用户设过类别的：照它们自己的规则。
        let marked = WorkSignals {
            eligible: false,
            ..serving
        };
        assert!(!w.observe(UPDATER, marked, RULE, 1_000));
    }
}
