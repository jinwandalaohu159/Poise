//! 把 IPC 请求分派到引擎的各个部分。只处理协议里列出的固定请求。

use poise_core::model::Pid;
use poise_core::prefs::Mode;
use poise_protocol::{ErrorCode, HelloResult, PROTOCOL_VERSION, Request, ResponseEnvelope};
use serde::Serialize;

use super::Engine;
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
                // 会话变了（切换用户），或者还不知道登录用户（服务启动时还没人登录）：重新找。
                if state.session_id != self.session.session_id()
                    || self.changes.store.user().is_none()
                {
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
                if mode == Mode::Vendor && !self.vendor_available() {
                    return reply::<()>(
                        id,
                        Err(RequestError::new(
                            ErrorCode::InvalidRequest,
                            "这台电脑没有机型极限模式",
                        )),
                    );
                }
                self.settings.mode = mode;
                self.save_settings();
                self.reschedule_now();
                ok(id, &())
            }
            Request::SetFanBoost { percent } => {
                self.settings.fan_boost = percent.min(100);
                self.save_settings();
                self.apply_fan_boost();
                ok(id, &())
            }
            Request::SetGameMode { mode } => {
                self.settings.game_mode = mode;
                self.save_settings();
                // 正在玩游戏时立即换到新的目标模式。
                self.reschedule_now();
                ok(id, &())
            }
            Request::SetSmartSchedule { enabled } => {
                self.settings.smart_schedule = enabled;
                self.save_settings();
                // 关闭时立即撤销全部进程调整，打开时立即按类别调整。
                self.reschedule_now();
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
            Request::CleanupOverview => reply(id, self.cleanup_overview().ok_or_else(not_ready)),
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
            Request::Diagnose => reply(id, self.diagnose().ok_or_else(not_ready)),
            Request::Trend => ok(id, &self.trend()),
        }
    }
}
