//! 请求失败的原因：给界面的分类代码 + 给日志的详细文字。

use poise_protocol::ErrorCode;

use super::changes::ChangeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestError {
    pub code: ErrorCode,
    pub detail: String,
}

impl RequestError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl From<ChangeError> for RequestError {
    fn from(e: ChangeError) -> Self {
        let code = match e {
            ChangeError::DryRun => ErrorCode::ObserveOnly,
            ChangeError::Journal(_) => ErrorCode::Failed,
        };
        Self::new(code, e.to_string())
    }
}
