//! 每个请求由谁回答：
//! - 日志与使用记录：日志系统直接回答，不等引擎的锁（界面看日志时不会和调度抢）；
//! - 一键清理、列出优化项：在引擎锁内只取要用的东西（很快），慢的执行和读取在锁外，
//!   采样、前台切换、托盘的上报都不用等它们；
//! - 其余：交给引擎（要等它的锁，采样进行中时会等一会儿）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use poise_core::model::Pid;
use poise_logbook::Logbook;
use poise_protocol::{CleanupResult, ErrorCode, Request, ResponseEnvelope};

use crate::engine::Engine;

/// 一次请求（含等待引擎）超过这个时间就记一条日志，用来发现界面操作"不跟手"的原因。
const SLOW_REQUEST: Duration = Duration::from_millis(100);

/// 修改优化项时，这么久之内枚举的目录可以直接用（项目的标识不会变）。
const CATALOG_REUSE_FOR_CHANGES: Duration = Duration::from_secs(60);

pub(super) fn answer(
    request: Request,
    id: u64,
    pid: Pid,
    engine: &Mutex<Engine>,
    logbook: &Logbook,
) -> ResponseEnvelope {
    match request {
        Request::Events { after } => ResponseEnvelope::ok(id, &logbook.events_after(after)),
        Request::Records => ResponseEnvelope::ok(id, &logbook.records()),
        Request::CleanNow => clean_now(engine, id),
        Request::ListOptimizations => list_optimizations(engine, id),
        request => ask_engine(engine, pid, id, request),
    }
}

fn internal(id: u64) -> ResponseEnvelope {
    ResponseEnvelope::error(id, ErrorCode::Internal, "服务内部错误")
}

/// 交给引擎处理。修改优化项要用的目录先在锁外枚举好（读注册表和插件的版本信息，可能要几百毫秒）。
fn ask_engine(engine: &Mutex<Engine>, pid: Pid, id: u64, request: Request) -> ResponseEnvelope {
    let started = Instant::now();
    let changes_optimizations = matches!(
        request,
        Request::ApplyOptimization { .. }
            | Request::RevertOptimization { .. }
            | Request::ApplyRecommended
    );
    if changes_optimizations && let Ok((catalog, user)) = engine.lock().map(|e| e.catalog_handle())
    {
        catalog.refresh(user.as_ref(), CATALOG_REUSE_FOR_CHANGES);
    }
    let name = request_name(&request);
    let (response, waited) = match engine.lock() {
        Ok(mut eng) => {
            let waited = started.elapsed();
            (eng.handle(id, pid, request), waited)
        }
        Err(_) => return internal(id),
    };
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

/// 一键清理：生成计划、记录结果在引擎锁内，执行在锁外（重启服务可能要等十几秒）。
fn clean_now(engine: &Mutex<Engine>, id: u64) -> ResponseEnvelope {
    let Ok(manual) = engine.lock().map(|mut e| e.begin_clean_now()) else {
        return internal(id);
    };
    let manual = match manual {
        Ok(Some(manual)) => manual,
        Ok(None) => return ResponseEnvelope::ok(id, &CleanupResult::default()),
        Err(e) => return ResponseEnvelope::error(id, e.code, e.detail),
    };
    let done = manual.job.run();
    // 执行完就让出执行权：服务停止时在等它（拿着引擎锁），不能等到再拿引擎锁之后。
    drop(manual.turn);
    match engine.lock() {
        Ok(mut e) => ResponseEnvelope::ok(id, &e.finish_clean_now(manual.summary, done)),
        Err(_) => internal(id),
    }
}

/// 列出优化项：重新枚举目录、读取各项的现状都在锁外（电源服务忙时读二三十项要好几秒）。
fn list_optimizations(engine: &Mutex<Engine>, id: u64) -> ResponseEnvelope {
    match engine.lock().map(|e| e.optimization_query()) {
        Ok(query) => ResponseEnvelope::ok(id, &query.run()),
        Err(_) => internal(id),
    }
}

/// 请求的名称（不含参数），用于日志。
fn request_name(request: &impl std::fmt::Debug) -> String {
    let text = format!("{request:?}");
    text.split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string()
}
