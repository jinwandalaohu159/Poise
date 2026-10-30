//! Poise 服务与界面之间的 IPC 协议。
//!
//! 传输：命名管道 [`PIPE_NAME`]，每条消息是一行 UTF-8 JSON。
//! 服务只接受下面列出的固定请求，不接受任何命令或任意路径去执行。

mod dto;
mod framing;

pub use dto::*;
pub use framing::{FramingError, MAX_MESSAGE_BYTES, read_message, write_message};

use serde::{Deserialize, Serialize};

use poise_core::model::{Pid, SessionState};
use poise_core::prefs::{AppPreference, GameMode, Mode};

pub const PIPE_NAME: &str = r"\\.\pipe\Poise";
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    /// 托盘代理上报用户会话状态（前台窗口、全屏、可见窗口、音频）。
    ReportSession(SessionState),
    /// 界面窗口是否正显示着（显示时服务才读取显卡数据）。显示期间需要定期重复上报。
    SetUiVisible {
        visible: bool,
    },
    GetStatus,
    /// "建议重启"的提示稍后再说：内核内存池没有再明显上涨之前不再提示。
    SnoozeRestartAdvice,
    SetMode {
        mode: Mode,
    },
    /// 玩游戏时切换到哪个模式。
    SetGameMode {
        mode: GameMode,
    },
    SetAutoClean {
        enabled: bool,
    },
    /// 智能调度开关。关闭时立即撤销全部进程调整。
    SetSmartSchedule {
        enabled: bool,
    },
    /// 风扇加速，0～100（0 = 不加速，由 BIOS 按温度调节）。超出范围按 100 处理。
    SetFanBoost {
        percent: u8,
    },
    CleanNow,
    /// 清理页：按程序列出占用，以及每一项会怎样处理。
    CleanupOverview,
    ListApps,
    /// `path` 只用作偏好的键，服务不会运行它。
    SetAppPreference {
        path: String,
        preference: AppPreference,
    },
    TempBoost {
        pid: Pid,
        minutes: u32,
    },
    ListOptimizations,
    ApplyOptimization {
        id: String,
    },
    RevertOptimization {
        id: String,
    },
    ApplyRecommended,
    RevertAll,
    Diagnose,
    /// 走势图的数据。只读、很轻，界面显示走势图时每 2 秒读取一次。
    Trend,
    /// 核心与缓存面板：核心域、缓存、负载，以及哪些程序放在哪里。只读，面板显示时每 2 秒读取一次。
    Cores,
    /// 日志里编号大于 `after` 的条目（最多几百条，最新的那些）；`after` 为 0 时取最新的一批。
    Events {
        after: u64,
    },
    /// 使用记录（每段有重点程序的时间一条），最新的在前。
    Records,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub id: u64,
    pub request: Request,
}

/// 失败原因的分类。界面按它显示对应文案（按界面语言）；`error` 里的文字只用于日志。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    /// 观察模式下不执行任何修改。
    ObserveOnly,
    NotFound,
    /// 目标程序已经退出。
    ProcessGone,
    InvalidPath,
    /// 数据还在收集中。
    NotReady,
    /// 系统拒绝了修改（例如权限不足、被其他程序占用）。
    Failed,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ResponseEnvelope {
    pub fn ok<T: Serialize>(id: u64, value: &T) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => Self {
                id,
                ok: true,
                result: Some(v),
                code: None,
                error: None,
            },
            Err(e) => Self::error(id, ErrorCode::Internal, e.to_string()),
        }
    }

    pub fn error(id: u64, code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            code: Some(code),
            error: Some(detail.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_use_stable_snake_case_json() {
        let r = RequestEnvelope {
            id: 7,
            request: Request::SetMode { mode: Mode::Full },
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"id":7,"request":{"method":"set_mode","params":{"mode":"full"}}}"#
        );
        let unit = serde_json::to_string(&Request::GetStatus).unwrap();
        assert_eq!(unit, r#"{"method":"get_status"}"#);
        let back: Request = serde_json::from_str(r#"{"method":"clean_now"}"#).unwrap();
        assert_eq!(back, Request::CleanNow);
    }

    #[test]
    fn unknown_methods_are_rejected() {
        assert!(
            serde_json::from_str::<Request>(r#"{"method":"run","params":{"cmd":"calc"}}"#).is_err()
        );
    }

    #[test]
    fn session_report_round_trips() {
        let s = SessionState {
            session_id: 1,
            foreground_pid: Some(42),
            fullscreen: true,
            visible_window_pids: vec![42, 7],
            ..Default::default()
        };
        let json = serde_json::to_string(&Request::ReportSession(s.clone())).unwrap();
        assert_eq!(
            serde_json::from_str::<Request>(&json).unwrap(),
            Request::ReportSession(s)
        );
    }
}
