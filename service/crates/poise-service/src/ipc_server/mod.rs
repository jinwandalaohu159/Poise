//! 命名管道服务端：每个连接一个线程，按请求交给日志系统或引擎（见 [`route`]）。
//!
//! 服务模式下额外校验客户端：必须是安装目录里的 Poise 程序。

mod route;

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use poise_core::model::Pid;
use poise_logbook::Logbook;
use poise_platform::ipc::pipe::{PipeConnection, PipeServer};
use poise_platform::win::handle::image_path;
use poise_platform::win::security::SERVICE_PIPE_SDDL;
use poise_protocol::{
    ErrorCode, FramingError, PIPE_NAME, RequestEnvelope, ResponseEnvelope, read_message,
    write_message,
};

use crate::config::RunMode;
use crate::engine::Engine;

/// 同时连接数上限，防止被大量连接拖垮。
const MAX_CONNECTIONS: usize = 8;

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(
        engine: Arc<Mutex<Engine>>,
        logbook: Logbook,
        mode: RunMode,
    ) -> anyhow::Result<Self> {
        let sddl = (mode == RunMode::Service).then_some(SERVICE_PIPE_SDDL);
        let server = PipeServer::new(PIPE_NAME, sddl)?;
        let allowed_dir = (mode == RunMode::Service)
            .then(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
            .flatten();
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let stop_flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ipc-accept".into())
            .spawn(move || {
                accept_loop(
                    &server,
                    &engine,
                    &logbook,
                    &stop_flag,
                    &active,
                    allowed_dir.as_ref(),
                )
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// 停止接受新连接：设置标志后自连一次，唤醒阻塞中的 accept。
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(PIPE_NAME);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn accept_loop(
    server: &PipeServer,
    engine: &Arc<Mutex<Engine>>,
    logbook: &Logbook,
    stop: &AtomicBool,
    active: &Arc<AtomicUsize>,
    allowed_dir: Option<&PathBuf>,
) {
    let mut failures = 0u32;
    loop {
        let conn = match server.accept() {
            Ok(c) => {
                failures = 0;
                c
            }
            Err(e) => {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                // 多是一时的（客户端连上就断开等）：等一会儿再接，不能让托盘和界面从此连不上。
                failures += 1;
                if failures.is_power_of_two() {
                    log::warn!("接受连接失败（连续第 {failures} 次）：{e}");
                }
                std::thread::sleep(ACCEPT_RETRY.saturating_mul(failures.min(50)));
                continue;
            }
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(pid) = conn.client_pid() else {
            continue;
        };
        if !client_allowed(pid, allowed_dir) {
            log::warn!("拒绝了来自 PID {pid} 的连接：不是 Poise 程序");
            continue;
        }
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let engine = engine.clone();
        let logbook = logbook.clone();
        let active = active.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("ipc-{pid}"))
            .spawn(move || {
                // 界面的请求（前台切换的通知也走这里）要及时处理。
                poise_platform::act::self_tune::responsive_thread();
                serve(conn, pid, &engine, &logbook);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            log::error!("无法创建连接线程");
        }
    }
}

fn client_allowed(pid: Pid, allowed_dir: Option<&PathBuf>) -> bool {
    let Some(dir) = allowed_dir else {
        // 开发模式：管道默认只允许当前用户连接。
        return true;
    };
    let path = image_path(pid).map(PathBuf::from);
    path.is_some_and(|p| p.parent().is_some_and(|parent| parent.eq(dir.as_path())))
}

/// 接受连接失败后，等这么久（连续失败时逐次加长，最长 5 秒）再接。
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

fn serve(conn: PipeConnection, pid: Pid, engine: &Mutex<Engine>, logbook: &Logbook) {
    // 连着的界面和托盘视为 Poise 自己，永不触碰。
    if let Ok(mut eng) = engine.lock() {
        eng.client_connected(pid);
    }
    let mut reader = BufReader::new(conn);
    loop {
        let envelope: RequestEnvelope = match read_message(&mut reader) {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(FramingError::Json(e)) => {
                let _ = write_message(
                    reader.get_mut(),
                    &ResponseEnvelope::error(
                        0,
                        ErrorCode::InvalidRequest,
                        format!("无效请求：{e}"),
                    ),
                );
                continue;
            }
            Err(e) => {
                log::debug!("连接 {pid} 断开：{e}");
                break;
            }
        };
        let response = route::answer(envelope.request, envelope.id, pid, engine, logbook);
        if write_message(reader.get_mut(), &response).is_err() {
            break;
        }
    }
    if let Ok(mut eng) = engine.lock() {
        eng.client_disconnected(pid);
    }
}
