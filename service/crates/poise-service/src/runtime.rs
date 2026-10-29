//! 控制台模式和服务模式共用的运行循环。

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};

use poise_platform::act::self_tune;

use crate::config::RunConfig;
use crate::engine::Engine;
use crate::ipc_server::IpcServer;

/// 运行直到 `stop` 收到信号（或发送端被丢弃）。
pub fn run(cfg: RunConfig, stop: Receiver<()>) -> anyhow::Result<()> {
    let mode = cfg.mode;
    let engine = Arc::new(Mutex::new(Engine::new(cfg)?));
    let ipc = IpcServer::start(engine.clone(), mode)?;
    // 采样与调度在这个线程里：每轮只用几毫秒，但不能排在被让路的后台程序后面。
    self_tune::responsive_thread();
    loop {
        let wait = match engine.lock() {
            Ok(mut e) => {
                e.tick();
                e.next_tick_in()
            }
            Err(_) => crate::engine::TICK,
        };
        match stop.recv_timeout(wait) {
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
