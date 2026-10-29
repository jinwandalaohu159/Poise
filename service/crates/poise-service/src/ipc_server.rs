//! 命名管道服务端：每个连接一个线程，请求交给引擎处理。
//!
//! 服务模式下额外校验客户端：必须是安装目录里的 Poise 程序。

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use poise_core::model::Pid;
use poise_platform::ipc::pipe::{PipeConnection, PipeServer};
use poise_platform::win::handle::image_path;
use poise_platform::win::security::SERVICE_PIPE_SDDL;
use poise_protocol::{
    ErrorCode, FramingError, PIPE_NAME, RequestEnvelope, ResponseEnvelope, read_message,
    write_message,
};

use crate::config::RunMode;
use crate::engine::Engine;
use crate::engine::catalog::Catalog;

/// 同时连接数上限，防止被大量连接拖垮。
const MAX_CONNECTIONS: usize = 8;

pub struct IpcServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(engine: Arc<Mutex<Engine>>, mode: RunMode) -> anyhow::Result<Self> {
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
                accept_loop(&server, &engine, &stop_flag, &active, allowed_dir.as_ref())
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
    stop: &AtomicBool,
    active: &Arc<AtomicUsize>,
    allowed_dir: Option<&PathBuf>,
) {
    loop {
        let conn = match server.accept() {
            Ok(c) => c,
            Err(e) => {
                log::error!("管道服务失败：{e}");
                return;
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
        let active = active.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("ipc-{pid}"))
            .spawn(move || {
                serve(conn, pid, &engine);
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

/// 一次请求（含等待引擎）超过这个时间就记一条日志，用来发现界面操作"不跟手"的原因。
const SLOW_REQUEST: Duration = Duration::from_millis(100);

/// 请求的名称（不含参数），用于日志。
fn request_name(request: &impl std::fmt::Debug) -> String {
    let text = format!("{request:?}");
    text.split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn serve(conn: PipeConnection, pid: Pid, engine: &Mutex<Engine>) {
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
        let started = Instant::now();
        let name = request_name(&envelope.request);
        if let Some(max_age) = Catalog::freshness_for(&envelope.request)
            && let Ok((catalog, user)) = engine.lock().map(|e| e.catalog_handle())
        {
            // 慢的枚举在引擎锁外进行。
            catalog.refresh(user.as_ref(), max_age);
        }
        let response = match engine.lock() {
            Ok(mut eng) => {
                let waited = started.elapsed();
                let response = eng.handle(envelope.id, pid, envelope.request);
                let total = started.elapsed();
                if total > SLOW_REQUEST {
                    log::info!(
                        "请求 {name} 耗时 {} ms（等待采样 {} ms）",
                        total.as_millis(),
                        waited.as_millis()
                    );
                }
                response
            }
            Err(_) => ResponseEnvelope::error(envelope.id, ErrorCode::Internal, "服务内部错误"),
        };
        if write_message(reader.get_mut(), &response).is_err() {
            break;
        }
    }
    if let Ok(mut eng) = engine.lock() {
        eng.client_disconnected(pid);
    }
}
