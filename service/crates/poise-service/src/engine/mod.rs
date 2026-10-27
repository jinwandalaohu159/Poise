//! 引擎：持有所有组件。各方面的行为分布在同目录的子模块中：
//!
//! - [`cycle`]：每个采样周期（采集 → 分类 → 调度 → 健康评估），并串起下面各部分
//! - [`mode`]：把实际模式落到电源计划与散热上，过热时自动退出全力
//! - [`cleanup`]：一键清理、延后清理、自动清理
//! - [`optimizer`]：系统优化项与游戏偏好
//! - [`status`]：给界面的状态、程序列表、诊断
//! - [`requests`]：把 IPC 请求分派到上面各部分

pub(crate) mod changes;
mod cleanup;
mod cycle;
mod error;
mod mode;

pub use cycle::TICK;
mod optimizer;
mod requests;
mod session;
mod status;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use poise_core::activity::{ActivityConfig, ActivityTracker};
use poise_core::classify::{Classification, Classifier, ClassifyConfig};
use poise_core::cleanup::{
    CleanupStep, KernelPoolConfig, KernelPoolWatch, LeakConfig, LeakDetector, LeakSuspect,
};
use poise_core::diagnose::{Health, HeatWatch};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::prefs::{Mode, Settings};
use poise_core::safety::GuardedExecutor;
use poise_core::schedule::{GameInfo, ScheduleConfig, Scheduler};
use poise_hardware::{Hardware, HardwareReading};
use poise_platform::act::executor::WinExecutor;
use poise_platform::act::launcher::{DirectLauncher, Launcher, UserSessionLauncher};
use poise_platform::act::self_tune;
use poise_platform::observe::process::snapshot::Snapshotter;
use poise_platform::observe::system;
use poise_platform::observe::topology::CpuTopology;
use poise_platform::persist::paths;
use poise_platform::persist::storage::JsonFile;
use poise_platform::settings::user::UserContext;
use poise_protocol::LastCleanup;

use crate::config::{RunConfig, RunMode};

use changes::Changes;
use session::SessionTracker;

/// 最近一个周期的结果，供状态查询、清理、诊断使用。
pub struct View {
    pub snapshot: Snapshot,
    pub classes: HashMap<Pid, Classification>,
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
    pub reading: HardwareReading,
    pub health: Health,
}

#[derive(Default)]
struct CleanupState {
    deferred: Vec<CleanupStep>,
    last_auto_at: Millis,
    suspects: Vec<LeakSuspect>,
    suspects_at: Option<Millis>,
    last: Option<LastCleanup>,
}

pub struct Engine {
    cfg: RunConfig,
    started: Instant,
    snapshotter: Snapshotter,
    activity: ActivityTracker,
    classifier: Classifier,
    scheduler: Scheduler,
    leaks: LeakDetector,
    kernel_pool: KernelPoolWatch,
    heat: HeatWatch,
    executor: GuardedExecutor,
    hardware: Hardware,
    changes: Changes,
    settings: Settings,
    settings_file: JsonFile<Settings>,
    session: SessionTracker,
    /// Poise 自己的进程（服务和已连接的界面），永不触碰。
    own_pids: HashSet<Pid>,
    cleanup: CleanupState,
    view: Option<View>,
}

impl Engine {
    pub fn new(cfg: RunConfig) -> anyhow::Result<Self> {
        paths::ensure_dir(&cfg.data_dir, cfg.mode == RunMode::Service)?;
        let topology = CpuTopology::detect();
        self_tune::apply(&topology);
        let launcher: Box<dyn Launcher> = match cfg.mode {
            RunMode::Console => Box::new(DirectLauncher),
            RunMode::Service => Box::new(UserSessionLauncher),
        };
        let user = match cfg.mode {
            RunMode::Console => UserContext::current_process_user(),
            RunMode::Service => None,
        };
        let settings_file = JsonFile::new(cfg.data_dir.join("settings.json"));
        let settings = settings_file.load().unwrap_or_default();
        log::info!(
            "Poise 启动：{:?} 模式，{}，大小核：{}",
            cfg.mode,
            if cfg.dry_run {
                "观察模式（不执行）"
            } else {
                "正常执行"
            },
            topology.is_hybrid()
        );
        Ok(Self {
            started: Instant::now(),
            snapshotter: Snapshotter::default(),
            activity: ActivityTracker::new(ActivityConfig::default()),
            classifier: Classifier::new(ClassifyConfig::default()),
            scheduler: Scheduler::new(ScheduleConfig::default()),
            leaks: LeakDetector::new(LeakConfig::default()),
            kernel_pool: KernelPoolWatch::new(KernelPoolConfig::default()),
            heat: HeatWatch::default(),
            executor: GuardedExecutor::new(
                Box::new(WinExecutor::new(topology, launcher)),
                cfg.dry_run,
            ),
            hardware: Hardware::detect(),
            changes: Changes::new(&cfg.data_dir, user, cfg.dry_run),
            settings,
            settings_file,
            session: SessionTracker::new(system::active_console_session()),
            own_pids: HashSet::from([std::process::id()]),
            cleanup: CleanupState::default(),
            view: None,
            cfg,
        })
    }

    fn now(&self) -> Millis {
        self.started.elapsed().as_millis() as Millis
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings_file.save(&self.settings) {
            log::error!("保存设置失败：{e}");
        }
    }

    /// 界面断开后不再视为 Poise 自己的进程。
    pub fn client_disconnected(&mut self, pid: Pid) {
        if pid != std::process::id() {
            self.own_pids.remove(&pid);
        }
    }

    /// 服务停止：撤销所有进程调整，还原硬件模式。系统优化项保留（它们是用户选择的）。
    pub fn shutdown(&mut self) {
        let actions = self.scheduler.release_all();
        if let Some(view) = &self.view {
            for planned in actions {
                if let Some(c) = view.classes.get(&planned.key.pid) {
                    let _ = self.executor.execute(
                        &planned.action,
                        c,
                        planned.original_priority,
                        poise_core::safety::Trigger::Auto,
                    );
                }
            }
        }
        let now = self.now();
        let Self {
            hardware, changes, ..
        } = self;
        let _ = changes.edit(|s, j| {
            hardware.restore(s, j, now);
            Ok(())
        });
        log::info!("Poise 已停止，进程调整与硬件模式已还原");
    }
}
