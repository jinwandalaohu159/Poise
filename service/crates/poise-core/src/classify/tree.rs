//! 进程树：一个进程属于哪个程序（辅助进程归到主程序，独立程序单独成树）。

use std::collections::{HashMap, HashSet};

use super::ClassifyInput;
use crate::known;
use crate::model::{Pid, ProcessKey, ProcessSample, SessionState, Snapshot};
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

/// 按程序路径记住的"是否在不参与调度的目录里"：路径的结论不会变，同一个程序文件常有几十个进程
/// （浏览器、聊天软件），每轮采样又都要判断，所以每个路径只查一次名单，跨轮次保留。
#[derive(Debug, Default)]
pub(super) struct NeverScheduledPaths(HashMap<String, bool>);

impl NeverScheduledPaths {
    /// 记住的路径太多时清空重来（程序装了又卸，路径不会无限增长，这只是兜底）。
    const LIMIT: usize = 4096;

    fn check(&mut self, path: &str) -> bool {
        if let Some(&listed) = self.0.get(path) {
            return listed;
        }
        if self.0.len() >= Self::LIMIT {
            self.0.clear();
        }
        let listed = known::is_never_scheduled_path(path);
        self.0.insert(path.to_string(), listed);
        listed
    }
}

impl Marks {
    pub fn new(snap: &Snapshot, session: &SessionState, paths: &mut NeverScheduledPaths) -> Self {
        let never_scheduled = snap
            .processes
            .iter()
            .filter(|p| {
                known::is_never_scheduled(&p.facts.name)
                    || p.facts
                        .image_path
                        .as_deref()
                        .is_some_and(|path| paths.check(path))
            })
            .map(|p| p.facts.key.pid)
            .collect();
        Self {
            visible: session.visible_window_pids.iter().copied().collect(),
            never_scheduled,
        }
    }
}

/// 每个进程上一轮所属程序的主进程。中间的进程退出后（例如命令行工具拉起编译服务进程后自己退出了，
/// 启动器拉起子进程后退出），留下的进程仍归原来的程序——它们干的是那个程序的活，用户在用那个程序时
/// 它们不该被当成后台程序让路。原来的程序也退出了，它们才各自成为独立的程序。
#[derive(Debug, Default)]
pub(super) struct Lineage(HashMap<ProcessKey, ProcessKey>);

impl Lineage {
    /// 父进程已经不在了：归到它上一轮所属的程序（那个程序还在、同一会话、它自己也不是独立程序时）。
    fn adopt(
        &self,
        p: &ProcessSample,
        by_pid: &HashMap<Pid, &ProcessSample>,
        input: &ClassifyInput,
        marks: &Marks,
    ) -> Option<Pid> {
        let root = self.0.get(&p.facts.key)?;
        let r = by_pid.get(&root.pid).filter(|r| r.facts.key == *root)?;
        let same_session = r.facts.session_id != 0 && r.facts.session_id == p.facts.session_id;
        (same_session && !is_independent(p, r, input, marks)).then_some(root.pid)
    }

    /// 记下这一轮各进程所属的程序（只记属于别的进程的；退出了的自然忘掉）。
    pub fn remember(&mut self, roots: &HashMap<Pid, Pid>, by_pid: &HashMap<Pid, &ProcessSample>) {
        let key = |pid: &Pid| by_pid.get(pid).map(|p| p.facts.key);
        self.0 = roots
            .iter()
            .filter(|(pid, root)| pid != root)
            .filter_map(|(pid, root)| Some((key(pid)?, key(root)?)))
            .collect();
    }
}

/// 每个进程所属进程树的根（见 [`parent_in_tree`]）。
/// 沿途经过的进程一起记下结果，每个进程只判断一次，整体是线性的。
pub(super) fn roots(
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    marks: &Marks,
    lineage: &Lineage,
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
            match parent_in_tree(current, by_pid, input, marks, lineage) {
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

/// 进程在同一棵树里的父进程。遇到桌面外壳、会话 0、跨会话，或者自己就是一个独立程序
/// （见 [`is_independent`]）时为空：它自己就是根。父进程已经退出（或进程号被复用）时，
/// 按它上一轮所属的程序算（见 [`Lineage`]）。
fn parent_in_tree(
    pid: Pid,
    by_pid: &HashMap<Pid, &ProcessSample>,
    input: &ClassifyInput,
    marks: &Marks,
    lineage: &Lineage,
) -> Option<Pid> {
    let proc_ = by_pid.get(&pid)?;
    let parent_pid = proc_.facts.parent_pid?;
    let parent = match by_pid.get(&parent_pid) {
        Some(parent) if parent.is_parent_of(proc_) => parent,
        _ => return lineage.adopt(proc_, by_pid, input, marks),
    };
    let stop = Some(parent_pid) == input.session.shell_pid
        || parent.facts.session_id == 0
        || parent.facts.session_id != proc_.facts.session_id
        || is_independent(proc_, parent, input, marks);
    (!stop).then_some(parent_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_results_are_remembered_and_the_cache_stays_bounded() {
        let mut paths = NeverScheduledPaths::default();
        let normal = r"d:\apps\editor.exe";
        assert!(!paths.check(normal));
        assert_eq!(paths.0.get(normal), Some(&false));
        for i in 0..NeverScheduledPaths::LIMIT + 10 {
            paths.check(&format!(r"d:\apps\{i}.exe"));
        }
        assert!(paths.0.len() <= NeverScheduledPaths::LIMIT);
    }
}
