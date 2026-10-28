//! 进程树：一个进程属于哪个程序（辅助进程归到主程序，独立程序单独成树）。

use std::collections::{HashMap, HashSet};

use super::ClassifyInput;
use crate::known;
use crate::model::{Pid, ProcessSample, SessionState, Snapshot};
use crate::prefs::AppPreference;

/// 子进程是不是一个独立的程序，而不是父程序的一部分：
/// - 输入法、驱动助手等不参与调度的程序：它们常被正在输入的程序拉起，但不属于那个程序；
/// - 前台全屏窗口所属的进程，以及已确认正在玩的游戏（从启动器里打开的游戏）；
/// - 用户单独为它设置过类别的程序（例如把客户端里的游戏本体标为"游戏"）；
/// - 自己有窗口、并且装在父程序目录之外的程序（例如从聊天软件里点链接打开的浏览器）。
///
/// 同一个程序的界面进程（例如 Steam 的 steamwebhelper）装在同一目录下，仍归到主程序。
fn is_independent(
    p: &ProcessSample,
    parent: &ProcessSample,
    input: &ClassifyInput,
    marks: &Marks,
) -> bool {
    let session = input.session;
    let path = p.facts.image_path.as_deref();
    let pid = p.facts.key.pid;
    if marks.never_scheduled.contains(&pid)
        || (session.fullscreen && session.foreground_pid == Some(pid))
        || input.active_games.contains(&pid)
        || input.preferences.lookup(&p.facts) != AppPreference::Auto
    {
        return true;
    }
    if !marks.visible.contains(&pid) || p.facts.name == parent.facts.name {
        return false;
    }
    let parent_dir = parent
        .facts
        .image_path
        .as_deref()
        .and_then(|p| p.rfind('\\').map(|i| &p[..=i]));
    let inside_parent = path
        .zip(parent_dir)
        .is_some_and(|(p, dir)| p.starts_with(dir));
    !inside_parent
}

/// 分类前对每个进程算一次的标记。
pub(super) struct Marks {
    /// 有可见窗口的进程。
    pub visible: HashSet<Pid>,
    /// 名单里不参与调度的进程（见 [`known::is_never_scheduled`]）。
    pub never_scheduled: HashSet<Pid>,
}

impl Marks {
    pub fn new(snap: &Snapshot, session: &SessionState) -> Self {
        // 同一个程序文件常有几十个进程（浏览器、聊天软件），路径只查一次。
        let mut by_path: HashMap<&str, bool> = HashMap::new();
        let never_scheduled = snap
            .processes
            .iter()
            .filter(|p| {
                known::is_never_scheduled(&p.facts.name)
                    || p.facts.image_path.as_deref().is_some_and(|path| {
                        *by_path
                            .entry(path)
                            .or_insert_with(|| known::is_never_scheduled_path(path))
                    })
            })
            .map(|p| p.facts.key.pid)
            .collect();
        Self {
            visible: session.visible_window_pids.iter().copied().collect(),
            never_scheduled,
        }
    }
}

/// 每个进程所属进程树的根（见 [`parent_in_tree`]）。
/// 沿途经过的进程一起记下结果，每个进程只判断一次，整体是线性的。
pub(super) fn roots(
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    marks: &Marks,
) -> HashMap<Pid, Pid> {
    let mut roots: HashMap<Pid, Pid> = HashMap::with_capacity(by_pid.len());
    let mut chain = Vec::new();
    for &pid in by_pid.keys() {
        chain.clear();
        let mut current = pid;
        let root = loop {
            if let Some(&root) = roots.get(&current) {
                break root;
            }
            chain.push(current);
            // 层数上限：异常的父子关系不会让它一直走下去。
            if chain.len() > 32 {
                break current;
            }
            match parent_in_tree(current, by_pid, input, marks) {
                Some(parent) => current = parent,
                None => break current,
            }
        };
        for &p in &chain {
            roots.insert(p, root);
        }
    }
    roots
}

/// 进程在同一棵树里的父进程。遇到桌面外壳、会话 0、跨会话、PID 复用，
/// 或者自己就是一个独立程序（见 [`is_independent`]）时为空：它自己就是根。
fn parent_in_tree(
    pid: Pid,
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    marks: &Marks,
) -> Option<Pid> {
    let proc_ = by_pid.get(&pid)?;
    let parent_pid = proc_.facts.parent_pid?;
    let parent = by_pid.get(&parent_pid)?;
    let stop = Some(parent_pid) == input.session.shell_pid
        || parent.facts.session_id == 0
        || parent.facts.session_id != proc_.facts.session_id
        || !parent.is_parent_of(proc_)
        || is_independent(proc_, parent, input, marks);
    (!stop).then_some(parent_pid)
}
