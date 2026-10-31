//! 清理的执行。重启服务、结束进程、重启程序都可能要等好几秒（服务停止和启动各最多等 15 秒），
//! 不能拿着引擎锁做：采样、前台切换、界面的请求都要等它，托盘代理的上报也会被卡住。
//! 引擎生成计划（很快），这里在锁外执行，结果再交回引擎记录：
//! - 一键清理在发请求的连接线程里执行（见 `ipc_server`），执行完再回复；
//! - 自动清理和延后的清理在单独的线程里执行，引擎下一轮取回结果（见 [`Background`]）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use poise_core::action::{ExecError, ExecOutcome};
use poise_core::classify::Classification;
use poise_core::cleanup::CleanupStep;
use poise_core::model::PriorityClass;
use poise_core::safety::{GuardedExecutor, Trigger};

/// 要执行的清理步骤（已经和当时的采样核对过）。
pub struct CleanupJob {
    pub(super) executor: Arc<GuardedExecutor>,
    pub(super) trigger: Trigger,
    pub(super) steps: Vec<ReadyStep>,
}

/// 一个步骤，以及执行时安全守卫要看的分类与原始优先级；进程已经不在了时为空。
pub(super) struct ReadyStep {
    pub step: CleanupStep,
    pub target: Option<(Classification, PriorityClass)>,
}

/// 执行完的清理：每个步骤的结果。
pub struct CleanupDone {
    pub(super) trigger: Trigger,
    pub(super) outcomes: Vec<(CleanupStep, Result<ExecOutcome, ExecError>)>,
}

impl CleanupJob {
    /// 逐个执行（经过安全守卫；进程号被复用时执行器会发现，见 `ActionExecutor::execute`）。
    pub fn run(self) -> CleanupDone {
        let outcomes = self
            .steps
            .into_iter()
            .map(|s| {
                let result = match &s.target {
                    Some((class, priority)) => self.executor.execute(
                        s.step.key,
                        &s.step.action,
                        class,
                        *priority,
                        self.trigger,
                    ),
                    None => Err(ExecError::Gone),
                };
                (s.step, result)
            })
            .collect();
        CleanupDone {
            trigger: self.trigger,
            outcomes,
        }
    }
}

/// 一键清理的执行权：同一时间只执行一次（托盘和界面同时点也不会重复结束、重启同一批程序）；
/// 服务停止时等正在执行的做完（见 `Engine::shutdown`）。
#[derive(Clone, Default)]
pub(super) struct ManualSlot(Arc<AtomicBool>);

impl ManualSlot {
    /// 占用；已经有一次在执行时为空。
    pub fn take(&self) -> Option<ManualTurn> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ManualTurn(self.0.clone()))
    }

    /// 等正在执行的一键清理做完，最多 `timeout`；返回是否做完了。
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while self.0.load(Ordering::Acquire) {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        true
    }
}

/// 占着的一键清理执行权；丢弃时释放。
pub struct ManualTurn(Arc<AtomicBool>);

impl Drop for ManualTurn {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// 在单独的线程里执行的清理（自动清理、延后的清理），同一时间只有一个。
#[derive(Default)]
pub(super) struct Background {
    running: Option<Receiver<CleanupDone>>,
}

impl Background {
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    pub fn start(&mut self, job: CleanupJob) {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("cleanup".into())
            .spawn(move || {
                let _ = tx.send(job.run());
            });
        match spawned {
            Ok(_) => self.running = Some(rx),
            Err(e) => log::warn!("无法创建清理线程：{e}"),
        }
    }

    /// 执行完了就取回结果（不等待）。
    pub fn finished(&mut self) -> Option<CleanupDone> {
        let rx = self.running.as_ref()?;
        match rx.try_recv() {
            Ok(done) => {
                self.running = None;
                Some(done)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.running = None;
                None
            }
        }
    }
}
