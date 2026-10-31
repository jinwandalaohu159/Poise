//! 把 IPC 请求分派到引擎的各个部分。只处理协议里列出的固定请求。
//! 只读的请求先看有没有留着的回复（见 [`super::answers`]）；会改变状态的请求让留着的回复作废。

use poise_core::model::Pid;
use poise_protocol::{ErrorCode, HelloResult, PROTOCOL_VERSION, Request, ResponseEnvelope};
use serde::Serialize;

use super::Engine;
use super::answers::Query;
use super::error::RequestError;

fn ok<T: Serialize>(id: u64, value: &T) -> ResponseEnvelope {
    ResponseEnvelope::ok(id, value)
}

/// 刚启动、还没有第一轮采样结果。
fn not_ready() -> RequestError {
    RequestError::new(ErrorCode::NotReady, "正在收集数据，请稍后再试")
}

fn reply<T: Serialize>(id: u64, r: Result<T, RequestError>) -> ResponseEnvelope {
    match r {
        Ok(v) => ResponseEnvelope::ok(id, &v),
        Err(e) => {
            log::info!("请求失败（{:?}）：{}", e.code, e.detail);
            ResponseEnvelope::error(id, e.code, e.detail)
        }
    }
}

impl Engine {
    pub fn handle(&mut self, id: u64, client: Pid, request: Request) -> ResponseEnvelope {
        if self.stopped {
            return ResponseEnvelope::error(id, ErrorCode::NotReady, "服务正在停止");
        }
        // 只读的请求不改变状态；上报会话、界面显示与否只在前台切换时改变状态（那时会立即重新调度，
        // 见 `reschedule_now`）。其余的都让留着的回复作废。
        let keeps = Query::of(&request).is_some()
            || matches!(
                request,
                Request::Hello { .. } | Request::ReportSession(_) | Request::SetUiVisible { .. }
            );
        if !keeps {
            self.answers.invalidate();
        }
        match request {
            Request::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    return ResponseEnvelope::error(
                        id,
                        ErrorCode::VersionMismatch,
                        "协议版本不匹配",
                    );
                }
                ok(
                    id,
                    &HelloResult {
                        protocol: PROTOCOL_VERSION,
                        service_version: env!("CARGO_PKG_VERSION").into(),
                    },
                )
            }
            Request::ReportSession(state) => {
                // 会话变了（切换用户），或者还不知道登录用户（服务启动时还没人登录）：重新找。
                if state.session_id != self.session.session_id()
                    || self.changes.store.user().is_none()
                {
                    self.on_session_changed(state.session_id);
                }
                let now = self.now();
                // 前台变了，或者暂停中托盘代理又上报了（重新打开 Poise）：立即重新调度，不等下一轮采样。
                if self.session.report(state, client, now) || self.paused {
                    self.reschedule_now();
                }
                ok(id, &())
            }
            Request::SetUiVisible { visible } => {
                let now = self.now();
                self.session.set_ui_visible(visible, now);
                ok(id, &())
            }
            Request::GetStatus => self.answer(id, Query::Status),
            Request::SnoozeRestartAdvice => {
                self.kernel_pool.snooze();
                ok(id, &())
            }
            Request::SetMode { mode } => reply(id, self.set_mode(mode)),
            Request::SetFanBoost { percent } => {
                self.set_fan_boost(percent);
                ok(id, &())
            }
            Request::SetGameMode { mode } => {
                self.set_game_mode(mode);
                ok(id, &())
            }
            Request::SetSmartSchedule { enabled } => {
                self.set_smart_schedule(enabled);
                ok(id, &())
            }
            Request::SetAutoClean { enabled } => {
                self.set_auto_clean(enabled);
                ok(id, &())
            }
            Request::CleanupOverview => self.answer(id, Query::CleanupOverview),
            Request::ListApps => self.answer(id, Query::Apps),
            Request::SetAppPreference { path, preference } => {
                let r = self.set_app_preference(&path, preference);
                self.reschedule_now();
                reply(id, r)
            }
            Request::TempBoost { pid, minutes } => {
                let r = self.temp_boost(pid, minutes);
                reply(id, r)
            }
            Request::ApplyOptimization { id: item } => reply(id, self.apply_optimization(&item)),
            Request::RevertOptimization { id: item } => reply(id, self.revert_optimization(&item)),
            Request::ApplyRecommended => reply(id, self.apply_recommended()),
            Request::RevertAll => reply(id, self.revert_optimizations()),
            Request::Diagnose => self.answer(id, Query::Diagnose),
            Request::Trend => self.answer(id, Query::Trend),
            Request::Cores => self.answer(id, Query::Cores),
            // 日志与记录由日志系统直接回答，一键清理、列出优化项在引擎锁外进行（见 `ipc_server::route`）。
            Request::Events { .. }
            | Request::Records
            | Request::CleanNow
            | Request::ListOptimizations => {
                ResponseEnvelope::error(id, ErrorCode::InvalidRequest, "这个请求不由引擎直接处理")
            }
        }
    }

    /// 只读的请求：有留着的回复就直接给，没有时整理一份并留下。
    fn answer(&mut self, id: u64, query: Query) -> ResponseEnvelope {
        if let Some(cached) = self.answers.get(query, id) {
            return cached;
        }
        let response = match query {
            Query::Status => ok(id, &self.status()),
            Query::Apps => ok(id, &self.apps()),
            Query::CleanupOverview => reply(id, self.cleanup_overview().ok_or_else(not_ready)),
            Query::Diagnose => reply(id, self.diagnose().ok_or_else(not_ready)),
            Query::Trend => ok(id, &self.trend()),
            Query::Cores => ok(id, &self.core_map()),
        };
        self.answers.put(query, &response);
        response
    }
}
