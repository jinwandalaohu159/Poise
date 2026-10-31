//! 进程分类：按行为和系统属性判断一个进程"是什么"，以及调度可以对它做什么。
//!
//! 角色只靠系统属性判断；游戏只靠行为判断（全屏、正在用显卡渲染）。用到的名单（反作弊、虚拟机、
//! 不参与调度）都在 [`crate::known`]，只用于安全。系统进程一律交给系统，调度只管用户程序。

use crate::activity::ActivityTracker;
use crate::fast_hash::{HashMap, HashSet};
use crate::model::{Millis, Pid, SessionState, Snapshot};
use crate::prefs::{AppPreference, Preferences};

mod game;
mod role;
mod scope;
mod tree;

use game::renders_fullscreen;
use role::{AntiCheat, restartable_service, role_of};
use scope::{preference_of, schedule_scope};
use tree::{Lineage, Marks, NeverScheduledPaths, StoodAlone, roots};

/// 进程的角色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// 带"关键进程"标记，结束它会蓝屏。
    Critical,
    /// PPL 等受保护进程。
    Protected,
    /// 反作弊程序本身。
    AntiCheat,
    /// 虚拟机工作进程（WSL、Hyper-V）。
    VirtualMachine,
    /// Poise 自己。
    SelfProcess,
    /// 桌面窗口合成器，结束后系统会自动重建。
    Compositor,
    /// 承载 Windows 服务的进程。
    ServiceHost,
    /// 由单一服务拉起的系统辅助进程（例如音频引擎），处理方式是重启该服务。
    ServiceChild { service: String },
    /// 用户会话里的普通程序。
    UserApp,
    /// 其他系统进程。
    SystemOther,
}

/// 调度可以对这个进程做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleScope {
    /// 不调整：系统进程与服务、受保护进程，以及输入/显示/声音/安全相关的程序。
    Excluded,
    /// 用户程序：按类别调整核心、优先级和节能。
    App,
    /// Windows 自己的维护程序（更新安装、搜索索引等，见 `known::is_maintenance_worker`）：只在有重点程序时
    /// 挪到副域、降低内存与磁盘优先级；不改优先级、不节能、不重启。
    Maintenance,
}

/// 分类结果：角色加上与用户感知相关的标记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub role: Role,
    /// 进程树的根（例如浏览器的各个子进程都归到浏览器主进程）。
    pub root_pid: Pid,
    pub in_foreground_tree: bool,
    pub has_visible_window: bool,
    /// 同一进程树里有可见窗口（例如终端里运行的命令行程序、浏览器的辅助进程）。
    /// 这样的进程是某个窗口程序的一部分，重启它会破坏那个程序。
    pub tree_has_window: bool,
    /// 同一进程树里有程序正在发声（例如浏览器由单独的子进程输出声音、由渲染进程解码）。
    pub plays_audio: bool,
    /// 这个进程本身在输出声音（[`Self::plays_audio`] 按整个进程树算）。
    pub emits_audio: bool,
    pub recently_active: bool,
    pub preference: AppPreference,
    /// 是否像游戏：用户标记，或者是前台全屏窗口所属的进程且正在用显卡渲染。
    /// 后者要持续一段时间才确认为"正在玩"（见 `schedule` 的游戏跟踪）。
    pub is_game: bool,
    /// 系统中存在内核级反作弊。此时游戏只允许"任务管理器等价"的操作。
    pub anti_cheat_present: bool,
    /// 可以安全重启的服务：只承载一个服务、接受停止、没有正在运行的依赖者。
    pub restartable_service: Option<String>,
    pub schedule: ScheduleScope,
}

impl Classification {
    /// 是否是绝对不能碰的进程。
    pub fn is_untouchable(&self) -> bool {
        matches!(
            self.role,
            Role::Critical
                | Role::Protected
                | Role::AntiCheat
                | Role::VirtualMachine
                | Role::SelfProcess
        )
    }

    /// 是否是调度可以按类别调整的用户程序。
    pub fn is_schedulable(&self) -> bool {
        self.schedule == ScheduleScope::App
    }

    /// 是否是 Windows 自己的维护程序（见 [`ScheduleScope::Maintenance`]）。
    pub fn is_maintenance(&self) -> bool {
        self.schedule == ScheduleScope::Maintenance
    }

    /// 用户此刻直接在用、或要求一直保持快的：前台的进程树，以及设为"始终优先"或"游戏"的程序。
    /// 调度策略（`schedule::policy::in_use`）和安全守卫都以它为准判断"正在用"，各自再加上自己看得到的情况。
    pub fn in_use_by_user(&self) -> bool {
        self.in_foreground_tree
            || matches!(
                self.preference,
                AppPreference::AlwaysFast | AppPreference::Game
            )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClassifyConfig {
    /// "最近在干活"的时间窗口。
    pub active_window_ms: Millis,
    /// 全屏程序的显卡 3D 占用达到这个百分比算在渲染游戏画面。
    /// 全屏视频的占用通常在个位数，游戏一般远高于此。
    pub game_gpu_pct: f32,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            active_window_ms: 3 * 60 * 1000,
            game_gpu_pct: 20.0,
        }
    }
}

/// 分类所需的全部上下文。
pub struct ClassifyInput<'a> {
    pub snapshot: &'a Snapshot,
    pub session: &'a SessionState,
    pub activity: &'a ActivityTracker,
    pub preferences: &'a Preferences,
    pub own_pids: &'a [Pid],
    /// 前台进程的显卡 3D 占用（只在全屏时测量）。
    pub foreground_gpu: Option<(Pid, f32)>,
    /// 已确认正在玩的游戏：各自是独立的程序，切到后台也不归到启动它的客户端名下。
    pub active_games: &'a [Pid],
    /// 界面代理最近是否上报过会话状态。没有时看不到用户桌面，
    /// 按最保守的方式处理：所有用户程序都视为"有窗口"，清理时一律不碰。
    pub session_fresh: bool,
}

pub struct Classifier {
    cfg: ClassifyConfig,
    never_scheduled_paths: NeverScheduledPaths,
    lineage: Lineage,
    stood_alone: StoodAlone,
}

impl Classifier {
    pub fn new(cfg: ClassifyConfig) -> Self {
        Self {
            cfg,
            never_scheduled_paths: NeverScheduledPaths::default(),
            lineage: Lineage::default(),
            stood_alone: StoodAlone::default(),
        }
    }

    pub fn classify_all(&mut self, input: &ClassifyInput) -> HashMap<Pid, Classification> {
        let snap = input.snapshot;
        let by_pid = snap.by_pid();
        let anti_cheat = AntiCheat::detect(snap);
        let marks = Marks::new(
            snap,
            input.session,
            &mut self.never_scheduled_paths,
            &self.stood_alone,
        );
        let roots = roots(&by_pid, input, &marks, &self.lineage);
        let Marks {
            visible,
            never_scheduled,
            ..
        } = marks;
        self.stood_alone.remember(&roots, &by_pid, input.session);
        self.lineage.remember(&roots, &by_pid);
        let audio: HashSet<Pid> = input.session.audio_pids.iter().copied().collect();
        let foreground_root = input
            .session
            .foreground_pid
            .and_then(|pid| roots.get(&pid).copied());
        // 发声按进程树算：输出声音的进程和为它解码、混音的进程同属一个程序。
        let audio_roots: HashSet<Pid> = audio
            .iter()
            .filter_map(|pid| roots.get(pid).copied())
            .collect();
        let windowed_roots: HashSet<Pid> = visible
            .iter()
            .filter_map(|pid| roots.get(pid).copied())
            .collect();

        snap.processes
            .iter()
            .map(|p| {
                let pid = p.facts.key.pid;
                let root_pid = roots[&pid];
                let role = role_of(p, &by_pid, input, &anti_cheat);
                let preference = preference_of(p, root_pid, &by_pid, input.preferences);
                let in_foreground_tree = foreground_root == Some(root_pid);
                let has_visible_window =
                    visible.contains(&pid) || (!input.session_fresh && p.facts.session_id != 0);
                // 已确认正在玩的游戏不再测显卡占用（见服务的 measure_foreground_gpu），按已确认的算。
                let is_game = role == Role::UserApp
                    && (preference == AppPreference::Game
                        || input.active_games.contains(&pid)
                        || renders_fullscreen(p, input, self.cfg.game_gpu_pct));
                let c = Classification {
                    schedule: schedule_scope(p, &role, never_scheduled.contains(&pid)),
                    root_pid,
                    in_foreground_tree,
                    has_visible_window,
                    tree_has_window: has_visible_window || windowed_roots.contains(&root_pid),
                    plays_audio: audio_roots.contains(&root_pid),
                    emits_audio: audio.contains(&pid),
                    recently_active: input.activity.is_recently_active(
                        &p.facts.key,
                        snap.at,
                        self.cfg.active_window_ms,
                    ),
                    preference,
                    is_game,
                    anti_cheat_present: anti_cheat.present(),
                    restartable_service: restartable_service(p, &role, &by_pid),
                    role,
                };
                (pid, c)
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests;
