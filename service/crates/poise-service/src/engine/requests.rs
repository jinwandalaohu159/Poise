//! 把 IPC 请求分派到引擎的各个部分。只处理协议里列出的固定请求。

use poise_core::model::{Pid, ProcessKey};
use poise_platform::settings::user::UserContext;
use poise_protocol::{ErrorCode, HelloResult, PROTOCOL_VERSION, Request, ResponseEnvelope};
use serde::Serialize;

use super::Engine;
use super::error::RequestError;
use crate::config::RunMode;

/// 临时全速的最长时间（分钟）。
const MAX_BOOST_MINUTES: u32 = 240;

fn ok<T: Serialize>(id: u64, value: &T) -> ResponseEnvelope {
    ResponseEnvelope::ok(id, value)
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
        match request {
            Request::Hello { protocol } => {
                // 已连接的界面和托盘视为 Poise 自己，永不触碰。
                self.own_pids.insert(client);
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
                if state.session_id != self.session.session_id() {
                    self.on_session_changed(state.session_id);
                }
                let now = self.now();
                if self.session.report(state, now) {
                    self.reschedule_now();
                }
                ok(id, &())
            }
            Request::SetUiVisible { visible } => {
                let now = self.now();
                self.session.set_ui_visible(visible, now);
                ok(id, &())
            }
            Request::GetStatus => {
                let status = self.status();
                ok(id, &status)
            }
            Request::SnoozeRestartAdvice => {
                self.kernel_pool.snooze();
                ok(id, &())
            }
            Request::SetMode { mode } => {
                self.settings.mode = mode;
                self.save_settings();
                self.reschedule_now();
                ok(id, &())
            }
            Request::SetAutoFullForGames { enabled } => {
                self.settings.auto_full_for_games = enabled;
                self.save_settings();
                ok(id, &())
            }
            Request::SetAutoClean { enabled } => {
                self.settings.auto_clean = enabled;
                self.save_settings();
                ok(id, &())
            }
            Request::CleanNow => {
                let result = self.clean_now();
                ok(id, &result)
            }
            Request::ListApps => ok(id, &self.apps()),
            Request::SetAppPreference { path, preference } => {
                let r = self.set_app_preference(&path, preference);
                self.reschedule_now();
                reply(id, r)
            }
            Request::TempBoost { pid, minutes } => {
                let r = self.temp_boost(pid, minutes);
                reply(id, r)
            }
            Request::ListOptimizations => ok(id, &self.list_optimizations()),
            Request::ApplyOptimization { id: item } => reply(id, self.apply_optimization(&item)),
            Request::RevertOptimization { id: item } => reply(id, self.revert_optimization(&item)),
            Request::ApplyRecommended => reply(id, self.apply_recommended()),
            Request::RevertAll => reply(id, self.revert_optimizations()),
            Request::Diagnose => reply(
                id,
                self.diagnose().ok_or_else(|| {
                    RequestError::new(ErrorCode::NotReady, "正在收集数据，请稍后再试")
                }),
            ),
        }
    }

    fn temp_boost(&mut self, pid: Pid, minutes: u32) -> Result<(), RequestError> {
        let key = self
            .process_key(pid)
            .ok_or_else(|| RequestError::new(ErrorCode::ProcessGone, "程序已经退出"))?;
        let until = self.now() + u64::from(minutes.min(MAX_BOOST_MINUTES)) * 60_000;
        self.scheduler.boost(key, until);
        self.reschedule_now();
        Ok(())
    }

    fn process_key(&self, pid: Pid) -> Option<ProcessKey> {
        self.view.as_ref()?.snapshot.find(pid).map(|p| p.facts.key)
    }

    /// 交互用户变化（登录、切换用户）时，更新"当前用户"的注册表位置。
    fn on_session_changed(&mut self, session_id: u32) {
        if self.cfg.mode == RunMode::Service {
            let user = UserContext::for_session(session_id);
            log::info!(
                "交互会话 {session_id}，用户：{:?}",
                user.as_ref().and_then(|u| u.sid.clone())
            );
            self.changes.store.set_user(user);
        }
    }
}
