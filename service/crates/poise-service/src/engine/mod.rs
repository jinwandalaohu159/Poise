//! 引擎：持有所有组件。各方面的行为分布在同目录的子模块中：
//!
//! - [`cycle`]：每个采样周期（采集 → 分类 → 调度 → 健康评估与处理器余量），并串起下面各部分；
//!   [`sampler`] 在引擎锁外采集进程快照，[`cadence`] 决定多久采一轮，[`phases`] 给每轮分段计时
//! - [`schedule`]：按调度计划调整进程、临时全速、停止时撤销，[`decision_events`] 把调度决定写成日志事件；
//!   [`adjusted_record`]：记下调整过的进程，异常退出后下次启动时恢复
//! - [`cores`]：核心与缓存面板（核心域、缓存、负载、程序放在哪里）
//! - 日志与使用记录交给独立的日志系统（`poise_logbook::Logbook`）：[`decision_events`] 把调度决定写成事件，
//!   [`round_summary`] 把每轮调度的结果整理成记录系统的输入。引擎只管交过去，不等、不写盘
//! - [`presence`]：托盘代理在线时才起作用，退出时撤销全部影响；[`clients`]：连着的界面和托盘
//! - [`auto`]：把用户模式（含"自动"）换算成具体模式
//! - [`mode`]：把实际模式落到电源计划与散热上（慢的硬件操作在硬件执行线程里），风扇加速；
//!   [`mode_attempts`]：等预读、失败后隔一阵再试
//! - [`cleanup`]：一键清理、延后清理、自动清理
//! - [`growth`]：与开机时、与进程稳定运行时相比的增长
//! - [`optimizer`]：系统优化项，[`catalog`] 是它们的目录（在请求线程里枚举）；
//!   [`power_guard`]：守住 Poise 写入的电源设置（被别处改掉、换了计划；读取在单独的线程里）；
//!   [`game_prefs`]：程序类别与游戏对应的系统设置
//! - [`changes`]：持久修改的唯一出口（设置存储 + 变更日志）
//! - [`session`]：用户会话、界面是否显示；[`user_settings`]：用户在界面上改的设置（模式、风扇、开关）
//! - [`status`]：给界面的状态、诊断；[`apps`]：程序列表；[`remembered`]：运行过的程序（保存在本机）
//! - [`requests`]：把 IPC 请求分派到上面各部分，[`answers`] 留着只读请求的回复（两轮采样之间不变），
//!   [`error`] 是请求失败的原因

mod adjusted_record;
mod answers;
mod apps;
mod auto;
mod cadence;
pub(crate) mod catalog;
pub(crate) mod changes;
mod cleanup;
mod clients;
mod convert;
mod cores;
mod cycle;
mod decision_events;
mod error;
mod game_prefs;
mod growth;
mod mode;
mod mode_attempts;
mod optimizer;
mod phases;
mod power_guard;
mod presence;
mod remembered;
mod requests;
mod round_summary;
mod sampler;
mod schedule;
mod session;
mod status;
#[cfg(test)]
mod tick_cost;
mod user_settings;

pub use cadence::TICK;
pub use sampler::Sampler;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use poise_core::activity::{ActivityConfig, ActivityTracker};
use poise_core::auto_mode::AutoMode;
use poise_core::classify::{Classification, Classifier, ClassifyConfig};
use poise_core::cleanup::{
    BootBaseline, GrowthConfig, GrowthWatch, KernelPoolConfig, KernelPoolWatch, LeakConfig,
    LeakDetector,
};
use poise_core::diagnose::{HeadroomWatch, Health, MetricHistory};
use poise_core::model::{Millis, Pid, Snapshot};
use poise_core::prefs::{Mode, Settings};
use poise_core::safety::GuardedExecutor;
use poise_core::schedule::{CoreLayout, GameInfo, ScheduleConfig, Scheduler};
use poise_hardware::power::CpuShape;
use poise_hardware::{Hardware, HardwareReading};
use poise_logbook::Logbook;
use poise_platform::act::executor::WinExecutor;
use poise_platform::act::launcher::{DirectLauncher, Launcher, UserSessionLauncher};
use poise_platform::act::self_tune;
use poise_platform::observe::system;
use poise_platform::observe::topology::CpuTopology;
use poise_platform::persist::paths;
use poise_platform::persist::storage::JsonFile;
use poise_platform::persist::writer::FileWriter;
use poise_platform::settings::user::UserContext;
use poise_protocol::Event;

use crate::config::{RunConfig, RunMode};
use crate::gpu_feed::GpuFeed;
use crate::sensor_feed::SensorFeed;

use catalog::Catalog;
use changes::Changes;
use session::SessionTracker;

/// 最近一个周期的结果，供状态查询、清理、诊断使用。
pub struct View {
    pub snapshot: Snapshot,
    pub classes: HashMap<Pid, Classification>,
    pub effective_mode: Mode,
    pub game: Option<GameInfo>,
    /// 有重点程序（见 `poise_core::schedule::SchedulePlan::focused`）。
    pub focused: bool,
    pub reading: HardwareReading,
    pub health: Health,
}

pub struct Engine {
    cfg: RunConfig,
    started: Instant,
    /// 采集进程快照（采样线程在引擎锁外用它，见 [`Sampler`]）。
    sampler: Sampler,
    /// 托盘代理不在（Poise 已退出）：不调度、不切换模式，影响已全部撤销。
    paused: bool,
    /// 服务正在停止：影响已撤销，不再调度、不再接受请求。
    stopped: bool,
    /// 前台全屏程序的显卡占用（识别游戏用），在独立线程里测量。
    gpu: GpuFeed,
    /// 最近一次测到的前台全屏程序及其 3D 占用。
    foreground_gpu: Option<(Pid, f32)>,
    activity: ActivityTracker,
    classifier: Classifier,
    scheduler: Scheduler,
    leaks: LeakDetector,
    kernel_pool: KernelPoolWatch,
    growth: GrowthWatch,
    baseline_file: JsonFile<BootBaseline>,
    /// 处理器还有没有散热与功耗的余量（最近一次健康评估的结果）：没有时调度让后台更早让路。
    headroom: HeadroomWatch,
    auto_mode: AutoMode,
    /// 最近几分钟的整机指标（主页与卡顿诊断的走势图）。
    history: MetricHistory,
    /// 处理器的大小核组成与核心域（设置页、核心与缓存面板显示，调度按它分配核心）。
    topology: CpuTopology,
    /// 执行进程调整与清理（清理在引擎锁外执行，共用同一个）。
    executor: Arc<GuardedExecutor>,
    hardware: Hardware,
    /// 硬件读数在采集线程里进行，这里只取最近一次的结果。
    sensors: SensorFeed,
    changes: Changes,
    /// 系统优化项的目录（在请求线程里刷新，见 `catalog`）。
    catalog: Arc<Catalog>,
    /// 守住 Poise 写入的电源设置（见 `power_guard`）。
    power_guard: power_guard::PowerGuard,
    settings: Settings,
    settings_file: JsonFile<Settings>,
    /// 在后台线程里保存设置、调整记录、运行过的程序（变更日志除外：它必须先落盘再改设置）。
    files: FileWriter,
    session: SessionTracker,
    /// 运行过的程序（程序偏好页"未运行"一栏）。
    seen: remembered::RememberedApps,
    /// 连着的界面和托盘（Poise 自己的进程，永不触碰）与各自的连接数：同一个进程可以有几条连接，
    /// 最后一条断开才算它走了。
    clients: clients::Clients,
    cleanup: cleanup::CleanupState,
    /// 观察模式下最近一次记录的硬件状态（只用于日志去重）。
    observed: mode::Observed,
    view: Option<View>,
    adjusted: adjusted_record::AdjustedRecord,
    /// 最近一次调整了程序、决定变了或前台切换的时间（决定采样快慢，见 `cycle`）。
    last_change_at: Millis,
    /// 上一轮采样的时刻（发现睡眠唤醒用，见 `cycle`）。
    last_tick_at: Option<Millis>,
    /// 切换模式的尝试：等预读、失败后隔一阵再试（见 `mode`）。
    mode_attempts: mode_attempts::ModeAttempts,
    /// 上一轮交给记录系统的摘要里有重点程序（见 `round_summary`）。
    round_focused: bool,
    /// 日志系统与记录系统（独立线程，见 `poise_logbook`）。
    logbook: Logbook,
    /// 定期在日志里写一次 Poise 自己的开销（见 `phases`）。
    cost_log: phases::CostLog,
    /// 只读请求留着的回复（见 `answers`）。
    answers: answers::Answers,
}

impl Engine {
    pub fn new(cfg: RunConfig, logbook: Logbook) -> anyhow::Result<Self> {
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
        let cpu = CpuShape {
            top_class: topology.top_class,
            uneven_cache: topology.uneven_cache(),
        };
        let mut hardware = Hardware::detect(cpu, &cfg.data_dir);
        let changes =
            Changes::new(&cfg.data_dir, user, cfg.dry_run).with_power(hardware.power_writer());
        let growth = GrowthWatch::new(
            GrowthConfig::default(),
            baseline_file.load(),
            growth::boot_unix_s(system::uptime_ms()),
        );
        logbook.event_with(
            Event::Started {
                version: env!("CARGO_PKG_VERSION").into(),
                observe_only: cfg.dry_run,
            },
            format!(
                "{:?} 方式运行，大核 {:?}，小核 {:?}，核心域（线程，物理核心） {:?}",
                cfg.mode,
                topology.performance_count,
                topology.efficiency_count,
                topology.domain_sizes()
            ),
        );
        let started = Instant::now();
        let files = FileWriter::start();
        let mut engine = Self {
            started,
            sampler: Sampler::new(started),
            paused: true,
            stopped: false,
            gpu: GpuFeed::start(TICK),
            foreground_gpu: None,
            activity: ActivityTracker::new(ActivityConfig::default()),
            classifier: Classifier::new(ClassifyConfig::default()),
            scheduler: Scheduler::new(ScheduleConfig {
                cores: CoreLayout::with_cores(&topology.domain_sizes()),
                ..Default::default()
            }),
            leaks: LeakDetector::new(LeakConfig::default()),
            kernel_pool: KernelPoolWatch::new(KernelPoolConfig::default()),
            growth,
            baseline_file,
            headroom: HeadroomWatch::default(),
            auto_mode: AutoMode::default(),
            history: MetricHistory::default(),
            executor: Arc::new(GuardedExecutor::new(
                Box::new(WinExecutor::new(topology.clone(), launcher)),
                cfg.dry_run,
            )),
            sensors: SensorFeed::start(hardware.take_sensors(), TICK),
            hardware,
            changes,
            catalog: Arc::default(),
            power_guard: power_guard::PowerGuard::start(),
            settings,
            settings_file,
            seen: remembered::RememberedApps::open(&cfg.data_dir, files.clone()),
            session: SessionTracker::new(system::active_console_session()),
            clients: clients::Clients::default(),
            cleanup: cleanup::CleanupState::default(),
            observed: mode::Observed::default(),
            view: None,
            adjusted: adjusted_record::AdjustedRecord::open(&cfg.data_dir, files.clone()),
            files,
            last_change_at: 0,
            last_tick_at: None,
            mode_attempts: mode_attempts::ModeAttempts::default(),
            round_focused: false,
            logbook,
            cost_log: phases::CostLog::default(),
            answers: answers::Answers::default(),
            topology,
            cfg,
        };
        engine.resync_game_settings();
        Ok(engine)
    }

    fn now(&self) -> Millis {
        self.started.elapsed().as_millis() as Millis
    }

    /// 在引擎锁外采集进程快照用（见 [`Sampler`]）。
    pub fn sampler(&self) -> Sampler {
        self.sampler.clone()
    }

    /// 界面或托盘连上了：视为 Poise 自己的进程（只接受安装目录里的程序连接，见 `ipc_server`）。
    pub fn client_connected(&mut self, pid: Pid) {
        self.clients.connected(pid);
    }

    /// 一条连接断开。这个进程的连接都断开后不再视为 Poise 自己的进程；是托盘代理时暂停。
    pub fn client_disconnected(&mut self, pid: Pid) {
        self.answers.invalidate();
        if self.clients.disconnected(pid) {
            self.on_client_gone(pid);
        }
    }

    /// 服务停止：撤销所有进程调整，还原硬件模式（见 [`Engine::withdraw`]）；之后不再调度、不再接受请求。
    /// 系统优化项保留（它们是用户选择的）。
    pub fn shutdown(&mut self, system_shutdown: bool) {
        self.stopped = true;
        if !system_shutdown {
            self.wait_manual_clean();
        }
        self.withdraw(if system_shutdown {
            presence::Withdraw::SystemShutdown
        } else {
            presence::Withdraw::Stop
        });
        if !self.files.flush(Duration::from_secs(5)) {
            log::warn!("停止前还有文件没保存完");
        }
        log::info!("Poise 已停止，进程调整与硬件模式已还原");
    }
}

/// 当前的 Unix 时间（毫秒）。
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
