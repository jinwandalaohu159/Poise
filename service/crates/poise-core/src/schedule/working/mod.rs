//! 用户的程序在干活（规则见 `trackers::work`），供 `judge` 使用，规则对所有程序都一样：
//!
//! - 归属：被用户直接用过、由这样的程序启动（本文件），或正在为用户的程序干活（经本机网络连着，
//!   见 [`serving`]）；
//! - 在干活时能用多少处理器（见 [`rooms`] 与 `policy::WorkRoom`）。
//!
//! 跨周期的状态都在 [`WorkState`] 里。

mod rooms;
mod serving;

use super::program::Program;
use super::trackers::{CoreShare, Pressure, Sighting, WorkSignals, WorkTracker};
use super::{ScheduleInput, Scheduler};
use crate::classify::Classification;
use crate::fast_hash::{HashMap, HashSet};
use crate::model::{Millis, Pid, ProcessKey, Snapshot};
use crate::prefs::AppPreference;

/// 和"用户的程序在干活"有关的跨周期状态。
#[derive(Debug, Default)]
pub(super) struct WorkState {
    /// 谁是用户的、谁在干活。
    tracker: WorkTracker,
    /// 有重点程序时留给它几个大核（见 `trackers::share`）。
    share: CoreShare,
    /// 有重点程序时处理器发热、被功耗或温度压着：到这个时刻之前，干活的程序只用小核（见 `rooms`）。
    squeezed_until: Millis,
    /// 上一轮正在用、在干活、是重点程序的程序（主进程号）：谁连着谁见 `serving`。
    wanted: HashSet<Pid>,
    /// 这一轮在干活的程序一共占用多少处理器（单核百分比）。
    core_pct: f32,
}

impl WorkState {
    pub(super) fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.tracker.retain_alive(alive);
    }

    /// 从头跟踪（Poise 暂停后继续）：只保留哪些程序被用户直接用过。
    pub(super) fn restarted(self) -> Self {
        let mut fresh = Self::default();
        fresh.tracker.restore(&self.tracker.engaged_keys());
        fresh
    }

    /// 被用户直接用过的程序（排好序）。
    pub(super) fn engaged_keys(&self) -> Vec<ProcessKey> {
        self.tracker.engaged_keys()
    }

    /// 被用户直接用过的程序变过几次（没变时不必再取 [`WorkState::engaged_keys`]）。
    pub(super) fn engaged_revision(&self) -> u64 {
        self.tracker.revision()
    }

    pub(super) fn restore(&mut self, keys: &[ProcessKey]) {
        self.tracker.restore(keys);
    }

    /// 这一轮在干活的程序一共占用几个核心。
    pub(super) fn cores(&self) -> f32 {
        self.core_pct / 100.0
    }
}

impl Scheduler {
    /// 记下程序有没有被用户直接用过（第一次看到它时查清是谁启动了它）。
    pub(super) fn note_engagement(
        &mut self,
        class: &Classification,
        program: &Program,
        schedulable: bool,
        input: &ScheduleInput,
    ) {
        let used = schedulable
            && (class.in_foreground_tree
                || program.boosted
                || class.preference == AppPreference::AlwaysFast);
        let root = program.root;
        let sighting = (!self.work.tracker.knows(&root)).then(|| Sighting {
            parent: self.parent_program(root, input.classes, input.snapshot),
        });
        self.work.tracker.note(root, used, sighting);
    }

    /// 程序是不是用户的、在干活（见 `trackers::work`）。正在玩的游戏、临时全速、用户设过类别的照它们自己的
    /// 规则；Windows 的维护程序不算。
    pub(super) fn working(
        &mut self,
        class: &Classification,
        program: &Program,
        schedulable: bool,
        input: &ScheduleInput,
    ) -> bool {
        let eligible = schedulable
            && !class.is_maintenance()
            && class.preference == AppPreference::Auto
            && !program.active_game
            && !program.pending_game
            && !program.boosted;
        let signals = WorkSignals {
            eligible,
            serving: program.serves,
            usage: self.load.of(&program.root),
            recent: self.load.recent(&program.root),
            busy: self
                .cfg
                .busy
                .thresholds(Pressure::Normal)
                .unwrap_or_default(),
        };
        self.work
            .tracker
            .observe(program.root, signals, self.cfg.work, input.snapshot.at)
    }

    /// 记下这一轮在干活的程序一共占用多少处理器（"自动"模式用，见 [`Scheduler::working_cores`]）。
    pub(super) fn sum_working_load(&mut self, programs: &HashMap<Pid, Program>) {
        self.work.core_pct = programs
            .values()
            .filter(|p| p.working)
            .map(|p| self.load.of(&p.root).core_pct)
            .sum();
    }

    /// 启动这个程序的程序（它的主进程的父进程所属的程序）。桌面外壳、系统组件从不算被用户直接用过
    /// （见 [`Scheduler::note_engagement`]），从它们启动的程序不会因此算在内。
    fn parent_program(
        &self,
        root: ProcessKey,
        classes: &HashMap<Pid, Classification>,
        snap: &Snapshot,
    ) -> Option<ProcessKey> {
        let p = snap.find(root.pid)?;
        let parent = snap.find(p.facts.parent_pid?)?;
        if !parent.is_parent_of(p) {
            return None;
        }
        let parent_root = classes.get(&parent.facts.key.pid)?.root_pid;
        (parent_root != root.pid)
            .then(|| self.load.root(parent_root))
            .flatten()
    }
}
