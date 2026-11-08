//! 正在为用户的程序干活的程序：经本机网络连着用户正在用、在干活、是重点程序的程序（见 `model::LocalLink`）。
//! 不看它是什么程序——浏览器连着的本地大模型、前端连着的开发服务器、脚本连着的数据库都一样。

use super::super::policy;
use super::super::program::Program;
use super::super::{ScheduleInput, Scheduler};
use crate::classify::Classification;
use crate::fast_hash::{HashMap, HashSet};
use crate::model::Pid;

impl Scheduler {
    /// 正在为用户的程序干活的程序（主进程号）：上一轮正在用、在干活、是重点程序的程序经本机网络连着它监听的
    /// 端口。一层层传下去：它自己在干活时，它连着的程序下一轮也算。
    pub(in super::super) fn serving_roots(&self, input: &ScheduleInput) -> HashSet<Pid> {
        input
            .snapshot
            .links
            .iter()
            .filter_map(|l| {
                let client = input.classes.get(&l.client)?.root_pid;
                let server = input.classes.get(&l.server)?.root_pid;
                (client != server && self.work.wanted.contains(&client)).then_some(server)
            })
            .collect()
    }

    /// 记下这一轮正在用、在干活、是重点程序的程序（下一轮看谁在为它们干活，见 [`Scheduler::serving_roots`]）。
    pub(in super::super) fn remember_wanted(
        &mut self,
        classes: &HashMap<Pid, Classification>,
        programs: &HashMap<Pid, Program>,
    ) {
        self.work.wanted = programs
            .iter()
            .filter(|(_, p)| {
                p.working
                    || p.focus.is_some()
                    || classes
                        .get(&p.class_pid)
                        .is_some_and(|c| policy::in_use(&p.view(c, false)))
            })
            .map(|(pid, _)| *pid)
            .collect();
    }
}
