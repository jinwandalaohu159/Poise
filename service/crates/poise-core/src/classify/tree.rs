//! 进程树：一个进程属于哪个程序（辅助进程归到主程序，独立程序单独成树）。

use std::collections::{HashMap, HashSet};

use super::ClassifyInput;
use crate::known;
use crate::model::{Pid, ProcessSample};

/// 子进程是不是一个独立的程序，而不是父程序的一部分：
/// - 输入法、驱动助手等不参与调度的程序：它们常被正在输入的程序拉起，但不属于那个程序；
/// - 前台全屏窗口所属的进程，以及已确认正在玩的游戏（从启动器里打开的游戏）；
/// - 自己有窗口、并且装在父程序目录之外的程序（例如从聊天软件里点链接打开的浏览器）。
///
/// 同一个程序的界面进程（例如 Steam 的 steamwebhelper）装在同一目录下，仍归到主程序。
pub(super) fn is_independent(
    p: &ProcessSample,
    parent: &ProcessSample,
    input: &ClassifyInput,
    visible: &HashSet<Pid>,
) -> bool {
    let session = input.session;
    let path = p.facts.image_path.as_deref();
    if known::is_never_scheduled(&p.facts.name) || path.is_some_and(known::is_never_scheduled_path)
    {
        return true;
    }
    let pid = p.facts.key.pid;
    if (session.fullscreen && session.foreground_pid == Some(pid))
        || input.active_games.contains(&pid)
    {
        return true;
    }
    if !visible.contains(&p.facts.key.pid) || p.facts.name == parent.facts.name {
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

/// 沿父进程向上找到进程树的根：遇到桌面外壳、会话 0、跨会话、PID 复用，
/// 或者自己就是一个独立程序（见 [`is_independent`]）时停止。
pub(super) fn root_of(
    pid: Pid,
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    visible: &HashSet<Pid>,
) -> Pid {
    let session = input.session;
    let mut current = pid;
    for _ in 0..32 {
        let Some(proc_) = by_pid.get(&current) else {
            break;
        };
        let Some(parent_pid) = proc_.facts.parent_pid else {
            break;
        };
        let Some(parent) = by_pid.get(&parent_pid) else {
            break;
        };
        let stop = Some(parent_pid) == session.shell_pid
            || parent.facts.session_id == 0
            || parent.facts.session_id != proc_.facts.session_id
            || !parent.is_parent_of(proc_)
            || is_independent(proc_, parent, input, visible);
        if stop {
            break;
        }
        current = parent_pid;
    }
    current
}
