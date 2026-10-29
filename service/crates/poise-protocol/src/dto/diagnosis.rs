//! 走势图与卡顿诊断。

use serde::{Deserialize, Serialize};

use poise_core::diagnose::{HistoryPoint, StutterReport};

/// 卡顿诊断：结论与依据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    #[serde(flatten)]
    pub report: StutterReport,
    /// 诊断时智能调度是否开启（便于开关前后对比）。
    pub smart_schedule: bool,
}

/// 最近几分钟的走势（主页与卡顿诊断共用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trend {
    /// 横轴覆盖的秒数。
    pub window_secs: u32,
    /// 从早到晚；比横轴多几秒，走势图左端不留空。
    pub points: Vec<HistoryPoint>,
}
