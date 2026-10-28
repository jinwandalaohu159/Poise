//! 引擎：持有所有组件。各方面的行为分布在同目录的子模块中：
//!
//! - [`cycle`]：每个采样周期（采集 → 分类 → 调度 → 健康评估），并串起下面各部分
//! - [`schedule`]：按调度计划调整进程、临时全速、停止时撤销
//! - [`auto`]：把用户模式（含"自动"）换算成具体模式
//! - [`mode`]：把实际模式落到电源计划与散热上（过热时退出全力和极限模式），风扇加速
//! - [`cleanup`]：一键清理、延后清理、自动清理
//! - [`growth`]：与开机时、与进程稳定运行时相比的增长
//! - [`optimizer`]：系统优化项与游戏偏好
//! - [`changes`]：持久修改的唯一出口（设置存储 + 变更日志）
//! - [`session`]：用户会话、界面是否显示
//! - [`status`]：给界面的状态、诊断；[`apps`]：程序列表与运行过的程序（保存在本机）
//! - [`requests`]：把 IPC 请求分派到上面各部分，[`error`] 是请求失败的原因

mod apps;
mod auto;
pub(crate) mod changes;
mod cleanup;
mod convert;
mod cycle;
mod error;
mod growth;
mod mode;
mod optimizer;
mod requests;
mod schedule;
mod session;
mod status;

pub use cycle::TICK;

use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use poise_core::activity::{ActivityConfig, ActivityTracker};
use poise_core::auto_mode::AutoMode;
use poise_core::classify::{Classification, Classifier, ClassifyConfig};
use poise_core::cleanup::{
    BootBaseline, CleanupStep, Grown, GrowthConfig, GrowthWatch, KernelPoolConfig, KernelPoolWatch,
    LeakConfig, LeakDetector, LeakSuspect,
};
use poise_core::diagnose::{Health, Heat, HeatWatch, MetricHistory};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::prefs::{Mode, Settings};
use poise_core::safety::GuardedExecutor;
use poise_core::schedule::{GameInfo, ScheduleConfig, Scheduler};
use poise_core::seen_apps::SeenApps;
use poise_hardware::{Hardware, HardwareReading};
use poise_platform::act::executor::WinExecutor;
use poise_platform::act::launcher::{DirectLauncher, Launcher, UserSessionLauncher};
use poise_platform::act::self_tune;
use poise_platform::observe::core_load::CoreLoadMeter;
use poise_platform::observe::process::snapshot::Snapshotter;
use poise_platform::observe::system;
use poise_platform::observe::topology::CpuTopology;
use poise_platform::persist::paths;
use poise_platform::persist::storage::JsonFile;
use poise_platform::settings::user::UserContext;
use poise_protocol::LastCleanup;

use crate::config::{RunConfig, RunMode};
use crate::sensor_feed::SensorFeed;

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
    /// 与自身基线相比异常增长的进程，与泄漏嫌疑同时刷新。
    grown: Vec<Grown>,
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
    growth: GrowthWatch,
    baseline_file: JsonFile<BootBaseline>,
    heat: HeatWatch,
    /// 上一个周期的温度状态（"自动"模式的温度墙用）。
    last_heat: Heat,
    auto_mode: AutoMode,
    /// 最近几分钟的整机指标（主页与卡顿诊断的走势图）。
    history: MetricHistory,
    /// 处理器的大小核组成（设置页显示）。
    topology: CpuTopology,
    /// 每个周期读取大核、小核各自的负载，记入走势。
    core_load_meter: CoreLoadMeter,
    executor: GuardedExecutor,
    hardware: Hardware,
    /// 硬件读数在采集线程里进行，这里只取最近一次的结果。
    sensors: SensorFeed,
    changes: Changes,
    settings: Settings,
    settings_file: JsonFile<Settings>,
    session: SessionTracker,
    /// 运行过的程序（程序偏好页"未运行"一栏）。
    seen: SeenApps,
    seen_file: JsonFile<SeenApps>,
    /// Poise 自己的进程（服务和已连接的界面），永不触碰。
    own_pids: HashSet<Pid>,
    cleanup: CleanupState,
    /// 观察模式下最近一次记录的硬件状态（只用于日志去重）。
    observed: mode::Observed,
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
        // 服务模式：先按当前控制台会话找登录用户（托盘代理上报的通常就是这个会话）；
        // 还没有人登录时为空，之后代理上报时再找。
        let user = match cfg.mode {
            RunMode::Console => UserContext::current_process_user(),
            RunMode::Service => UserContext::for_session(system::active_console_session()),
        };
        let settings_file = JsonFile::new(cfg.data_dir.join("settings.json"));
        let settings = settings_file.load().unwrap_or_default();
        let baseline_file = JsonFile::new(cfg.data_dir.join(growth::BASELINE_FILE));
        let hardware = Hardware::detect();
        let seen_file = JsonFile::new(cfg.data_dir.join(apps::SEEN_FILE));
        let growth = GrowthWatch::new(
            GrowthConfig::default(),
            baseline_file.load(),
            growth::boot_unix_s(system::uptime_ms()),
        );
        log::info!(
            "Poise 启动：{:?} 模式，{}，大核 {:?}，小核 {:?}",
            cfg.mode,
            if cfg.dry_run {
                "观察模式（不执行）"
            } else {
                "正常执行"
            },
            topology.performance_count,
            topology.efficiency_count
        );
        let mut engine = Self {
            started: Instant::now(),
            snapshotter: Snapshotter::default(),
            activity: ActivityTracker::new(ActivityConfig::default()),
            classifier: Classifier::new(ClassifyConfig::default()),
            scheduler: Scheduler::new(ScheduleConfig {
                hybrid: topology.is_hybrid(),
                ..Default::default()
            }),
            leaks: LeakDetector::new(LeakConfig::default()),
            kernel_pool: KernelPoolWatch::new(KernelPoolConfig::default()),
            growth,
            baseline_file,
            heat: HeatWatch::default(),
            last_heat: Heat::Normal,
            auto_mode: AutoMode::default(),
            history: MetricHistory::default(),
            core_load_meter: CoreLoadMeter::new(&topology),
            executor: GuardedExecutor::new(
                Box::new(WinExecutor::new(topology.clone(), launcher)),
                cfg.dry_run,
            ),
            sensors: SensorFeed::start(hardware.sensors(), TICK),
            hardware,
            changes: Changes::new(&cfg.data_dir, user, cfg.dry_run),
            settings,
            settings_file,
            seen: seen_file.load().unwrap_or_default(),
            seen_file,
            session: SessionTracker::new(system::active_console_session()),
            own_pids: HashSet::from([std::process::id()]),
            cleanup: CleanupState::default(),
            observed: mode::Observed::default(),
            view: None,
            topology,
            cfg,
        };
        engine.prune_seen_apps();
        Ok(engine)
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
        self.release_processes();
        self.restore_hardware();
        log::info!("Poise 已停止，进程调整与硬件模式已还原");
    }
}

/// 当前的 Unix 时间（毫秒）。
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
