//! 最近几分钟的整机指标，供主页和卡顿诊断画走势图。只保存在内存里，固定长度。

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::Millis;

/// 走势图横轴：5 分钟。
pub const HISTORY_WINDOW_MS: Millis = 5 * 60_000;

/// 比横轴多保留的时间：走势图显示的"现在"比实际晚几秒（保证右端始终有数据），
/// 曲线又在两次刷新之间连续左移，左端需要窗口外的点补上，否则会露出空白。
const HISTORY_MARGIN_MS: Millis = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HistoryPoint {
    /// 距现在多少秒（越大越早），精确到毫秒：走势图实时左移时位置连续，不按整秒跳。
    pub secs_ago: f32,
    pub cpu_pct: f32,
    /// 大核、小核各自的平均占用；不是大小核处理器时为空。
    pub performance_pct: Option<f32>,
    pub efficiency_pct: Option<f32>,
    pub mem_pct: f32,
    pub cpu_temp: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub cpu_pct: f32,
    pub performance_pct: Option<f32>,
    pub efficiency_pct: Option<f32>,
    pub mem_pct: f32,
    pub cpu_temp: Option<f32>,
}

#[derive(Debug)]
pub struct MetricHistory {
    /// 保留的时长（横轴加上余量）。
    keep_ms: Millis,
    points: VecDeque<(Millis, Sample)>,
}

impl Default for MetricHistory {
    fn default() -> Self {
        Self::new(HISTORY_WINDOW_MS + HISTORY_MARGIN_MS)
    }
}

impl MetricHistory {
    pub fn new(keep_ms: Millis) -> Self {
        Self {
            keep_ms,
            points: VecDeque::new(),
        }
    }

    pub fn push(&mut self, now: Millis, sample: Sample) {
        self.points.push_back((now, sample));
        while self
            .points
            .front()
            .is_some_and(|(t, _)| now.saturating_sub(*t) > self.keep_ms)
        {
            self.points.pop_front();
        }
    }

    /// 从早到晚的全部点。
    pub fn recent(&self, now: Millis) -> Vec<HistoryPoint> {
        self.points
            .iter()
            .map(|(t, s)| HistoryPoint {
                secs_ago: now.saturating_sub(*t) as f32 / 1000.0,
                cpu_pct: s.cpu_pct,
                performance_pct: s.performance_pct,
                efficiency_pct: s.efficiency_pct,
                mem_pct: s.mem_pct,
                cpu_temp: s.cpu_temp,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(cpu: f32) -> Sample {
        Sample {
            cpu_pct: cpu,
            performance_pct: None,
            efficiency_pct: None,
            mem_pct: 50.0,
            cpu_temp: Some(60.0),
        }
    }

    #[test]
    fn keeps_only_the_window_oldest_first() {
        let mut h = MetricHistory::new(10_000);
        for i in 0..10u64 {
            h.push(i * 2_000, s(i as f32));
        }
        let points = h.recent(18_000);
        // 18 秒时只保留 8～18 秒之间的点。
        assert_eq!(
            points.iter().map(|p| p.cpu_pct).collect::<Vec<_>>(),
            vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );
        assert_eq!(points.first().unwrap().secs_ago, 10.0);
        assert_eq!(points.last().unwrap().secs_ago, 0.0);
    }
}
