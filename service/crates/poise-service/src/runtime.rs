//! 控制台模式和服务模式共用的运行循环。

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::RunConfig;
use crate::engine::{Engine, TICK};
use crate::ipc_server::IpcServer;

/// 一轮采样超过这个时间就记一条日志：采样期间引擎被锁住，前台切换和界面的请求要等它结束。
/// 硬件读数在采集线程里，正常一轮只要几十毫秒（刚启动时读取进程信息会慢一些）。
const SLOW_TICK: Duration = Duration::from_millis(150);

/// 运行直到 `stop` 收到信号（或发送端被丢弃）。
pub fn run(cfg: RunConfig, stop: Receiver<()>) -> anyhow::Result<()> {
    let mode = cfg.mode;
    let engine = Arc::new(Mutex::new(Engine::new(cfg)?));
    let ipc = IpcServer::start(engine.clone(), mode)?;
    loop {
        if let Ok(mut e) = engine.lock() {
            let started = Instant::now();
            e.tick();
            let took = started.elapsed();
            if took > SLOW_TICK {
                log::info!("本轮采样耗时 {} ms", took.as_millis());
            }
        }
        match stop.recv_timeout(TICK) {
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    ipc.stop();
    if let Ok(mut e) = engine.lock() {
        e.shutdown();
    }
    Ok(())
}
