//! 控制台模式和服务模式共用的运行循环。

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};

use poise_logbook::Logbook;
use poise_platform::act::self_tune;

use crate::config::RunConfig;
use crate::engine::Engine;
use crate::ipc_server::IpcServer;

/// 为什么停止。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 停止服务、卸载、控制台里按 Ctrl+C。
    Requested,
    /// 系统要关机或重启。
    SystemShutdown,
}

/// 运行直到 `stop` 收到信号（或发送端被丢弃）。停下调度之后调用 `stopping`（服务据此报告"正在停止"）。
pub fn run(
    cfg: RunConfig,
    stop: Receiver<StopReason>,
    stopping: impl FnOnce(),
) -> anyhow::Result<()> {
    let mode = cfg.mode;
    // 日志系统与记录系统在自己的线程里，引擎和命名管道服务端各拿一份接口。
    let logbook = Logbook::open(&cfg.data_dir);
    let engine = Engine::new(cfg, logbook.clone())?;
    let sampler = engine.sampler();
    let engine = Arc::new(Mutex::new(engine));
    let ipc = IpcServer::start(engine.clone(), logbook.clone(), mode)?;
    // 采样与调度在这个线程里：每轮只用几毫秒，但不能排在被让路的后台程序后面。
    self_tune::responsive_thread();
    let mut broken = false;
    let reason = loop {
        // 先在锁外采集进程快照（每轮最费时的一步），再锁住引擎处理它。
        let sample = sampler.sample();
        let wait = match engine.lock() {
            Ok(mut e) => {
                e.tick(sample);
                e.next_tick_in()
            }
            // 别的线程拿着引擎时崩溃了：引擎的状态可能不完整，不再调度。撤销 Poise 的影响后以失败退出，
            // 由服务管理器重启（见 `install`），而不是空转着什么都不做。
            Err(_) => {
                log::error!("调度引擎在请求线程里崩溃，撤销调整后退出，等服务管理器重启");
                broken = true;
                break StopReason::Requested;
            }
        };
        match stop.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(reason) => break reason,
            Err(RecvTimeoutError::Disconnected) => break StopReason::Requested,
        }
    };
    stopping();
    // 先停下引擎（之后不再调度、不再接受修改），再关管道：已连着的界面和托盘的请求会被拒绝，
    // 不会把刚还原的模式又写回去。崩溃过的引擎也要撤销：进程调整、电源设置、散热模式不能留下。
    engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shutdown(reason == StopReason::SystemShutdown);
    ipc.stop();
    // 服务停止前把日志和记录写完。
    logbook.flush();
    if broken {
        anyhow::bail!("调度引擎崩溃");
    }
    Ok(())
}
