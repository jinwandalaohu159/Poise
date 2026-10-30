//! 处理器实际的性能水平与限速：性能计数器 Processor Information（任务管理器"速度"的同一来源）。只读。
//!
//! 硬件自主调频的处理器（Intel 6 代起、AMD Zen 2 起）上，`CallNtPowerInformation` 报告的"当前频率"
//! 永远是基础频率，看不出睿频，也看不出降频。这两个计数器才是实际情况：
//! - `% Processor Performance`：相对基础频率的实际性能（睿频时超过 100）；
//! - `% Performance Limit`：固件允许的上限（温度墙、功耗墙限速时低于 100）。
//!
//! 每次读取得到与上一次之间的平均值，第一次没有可比较的数据。一次读取要 0.5～1.5 毫秒处理器时间
//! （要收集所有逻辑处理器的数据），所以至少隔 [`READ_EVERY_MS`] 才读一次——给的是平均，读得少不丢信息。

use poise_core::model::Millis;
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows::core::PCWSTR;

use crate::win::wide::WideString;

const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const LIMIT: &str = r"\Processor Information(_Total)\% Performance Limit";

/// 两次读取至少隔这么久（采样间隔为 2 秒时隔一轮读一次）。
pub const READ_EVERY_MS: Millis = 3_900;

/// 一次读数（百分比）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CpuPerformance {
    /// 相对基础频率的实际性能。
    pub performance_pct: f32,
    /// 固件允许的性能上限；100 表示没有限速。
    pub limit_pct: f32,
}

/// 打不开计数器（例如被管理员禁用了性能计数器）时一直返回 None。
pub struct CpuPerformanceMeter {
    query: Option<Query>,
    /// 上次读取的时间和读数。
    last: Option<(Millis, Option<CpuPerformance>)>,
}

impl Default for CpuPerformanceMeter {
    fn default() -> Self {
        Self {
            query: Query::open(),
            last: None,
        }
    }
}

impl CpuPerformanceMeter {
    /// 此刻的读数：距上次读取不到 [`READ_EVERY_MS`] 时沿用上次的。
    pub fn sample_at(&mut self, now: Millis) -> Option<CpuPerformance> {
        if let Some((at, reading)) = self.last
            && now.saturating_sub(at) < READ_EVERY_MS
        {
            return reading;
        }
        let reading = self.query.as_mut()?.read();
        self.last = Some((now, reading));
        reading
    }
}

struct Query {
    handle: PDH_HQUERY,
    performance: PDH_HCOUNTER,
    limit: PDH_HCOUNTER,
}

// SAFETY: PDH 句柄可以在任意线程使用；计量器只在持有引擎锁时被调用，不会并发访问。
unsafe impl Send for Query {}

impl Query {
    fn open() -> Option<Self> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: 输出到本地变量；之后出错时 query 被丢弃，查询随之关闭。
        unsafe {
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) != 0 {
                return None;
            }
            let mut query = Self {
                handle,
                performance: PDH_HCOUNTER::default(),
                limit: PDH_HCOUNTER::default(),
            };
            for (path, counter) in [
                (PERFORMANCE, &mut query.performance),
                (LIMIT, &mut query.limit),
            ] {
                let path = WideString::new(path);
                if PdhAddEnglishCounterW(handle, path.as_pcwstr(), 0, counter) != 0 {
                    return None;
                }
            }
            PdhCollectQueryData(handle);
            Some(query)
        }
    }

    fn read(&mut self) -> Option<CpuPerformance> {
        // SAFETY: 句柄在 Query 存活期间有效。
        if unsafe { PdhCollectQueryData(self.handle) } != 0 {
            return None;
        }
        Some(CpuPerformance {
            performance_pct: value(self.performance)?,
            limit_pct: value(self.limit)?,
        })
    }
}

fn value(counter: PDH_HCOUNTER) -> Option<f32> {
    let mut v = PDH_FMT_COUNTERVALUE::default();
    // SAFETY: 输出到本地变量。
    let status = unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut v) };
    let valid = matches!(v.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA);
    // SAFETY: 按 PDH_FMT_DOUBLE 取值。
    (status == 0 && valid).then_some(unsafe { v.Anonymous.doubleValue } as f32)
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: 关闭本对象打开的查询（同时释放其中的计数器）。
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_performance_and_limit_after_the_first_sample() {
        let mut meter = CpuPerformanceMeter::default();
        let first = meter.sample_at(0);
        std::thread::sleep(std::time::Duration::from_millis(300));
        // 隔得太近：沿用上次的读数，不再读。
        assert_eq!(meter.sample_at(READ_EVERY_MS - 1), first);
        let p = meter
            .sample_at(READ_EVERY_MS)
            .expect("读不到处理器性能计数器");
        assert!(
            p.performance_pct > 0.0 && p.performance_pct < 400.0,
            "{p:?}"
        );
        assert!(p.limit_pct > 0.0 && p.limit_pct <= 100.0, "{p:?}");
    }
}
