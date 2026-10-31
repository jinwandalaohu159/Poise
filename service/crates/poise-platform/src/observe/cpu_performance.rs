//! 处理器实际的性能水平与限速：性能计数器 Processor Information（任务管理器"速度"的同一来源）。只读。
//!
//! 硬件自主调频的处理器（Intel 6 代起、AMD Zen 2 起）上，`CallNtPowerInformation` 报告的"当前频率"
//! 永远是基础频率，看不出睿频，也看不出降频。这两个计数器才是实际情况：
//! - `% Processor Performance`：相对基础频率的实际性能（睿频时超过 100）；
//! - `% Performance Limit`：固件允许的上限（温度墙、功耗墙限速时低于 100）。
//!
//! 每次读取得到与上一次之间的平均值，第一次没有可比较的数据。一次读取要 0.5～1.5 毫秒处理器时间
//! （要收集所有逻辑处理器的数据），所以至少隔 [`READ_EVERY_MS`] 才读一次——给的是平均，读得少不丢信息。
//! 系统忙的时候一次读取实测要两三百毫秒，所以在自己的线程里读：采样只取最近一次的读数，从不等待。

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

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

/// 读取线程的句柄，丢弃时线程结束。打不开计数器（开机早期还没就绪，或被管理员禁用了性能计数器）时
/// 返回 None，之后最多每 [`REOPEN_EVERY`] 再试一次。
pub struct CpuPerformanceMeter {
    /// 让读取线程读一次。
    read: Sender<()>,
    /// 最近一次的读数。
    latest: Arc<Mutex<Option<CpuPerformance>>>,
    /// 上次让它读的时间。
    requested_at: Option<Millis>,
}

impl CpuPerformanceMeter {
    /// 启动读取线程。
    pub fn start() -> Self {
        let (read, requests) = mpsc::channel::<()>();
        let latest = Arc::new(Mutex::new(None));
        let shared = latest.clone();
        let spawned = std::thread::Builder::new()
            .name("cpu-perf".into())
            .spawn(move || {
                // 是否被限速决定要不要让路：小核被挤满时也要按时读到。
                crate::act::self_tune::responsive_thread();
                let mut query = None;
                let mut opened_at: Option<std::time::Instant> = None;
                for () in requests {
                    if query.is_none() && opened_at.is_none_or(|t| t.elapsed() >= REOPEN_EVERY) {
                        opened_at = Some(std::time::Instant::now());
                        query = Query::open();
                    }
                    let reading = query.as_mut().and_then(Query::read);
                    if let Ok(mut l) = shared.lock() {
                        *l = reading;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("创建处理器性能读取线程失败：{e}");
        }
        Self {
            read,
            latest,
            requested_at: None,
        }
    }

    /// 最近一次的读数（不等待）；距上次读取够 [`READ_EVERY_MS`] 时让读取线程再读一次，下一轮可见。
    pub fn sample_at(&mut self, now: Millis) -> Option<CpuPerformance> {
        if self
            .requested_at
            .is_none_or(|at| now.saturating_sub(at) >= READ_EVERY_MS)
        {
            self.requested_at = Some(now);
            let _ = self.read.send(());
        }
        self.latest()
    }

    /// 最近一次的读数（不等待），不触发读取。
    pub fn latest(&self) -> Option<CpuPerformance> {
        self.latest.lock().ok().and_then(|l| *l)
    }
}

struct Query {
    handle: PDH_HQUERY,
    performance: PDH_HCOUNTER,
    limit: PDH_HCOUNTER,
    /// 上次采集的时间。处理器性能是两次采集之间的平均值，隔得太近（刚打开就读）算出来不准。
    collected_at: std::time::Instant,
}

/// 两次采集至少隔这么久。
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
/// 打不开计数器时，多久再试一次。
const REOPEN_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

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
                collected_at: std::time::Instant::now(),
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
        // 在读取线程里，等一下不影响别人。
        std::thread::sleep(MIN_INTERVAL.saturating_sub(self.collected_at.elapsed()));
        self.collected_at = std::time::Instant::now();
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
    fn reads_in_the_background_and_never_waits() {
        let mut meter = CpuPerformanceMeter::start();
        let started = std::time::Instant::now();
        let _ = meter.sample_at(0);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        let mut reading = None;
        while reading.is_none() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(20));
            reading = meter.latest();
        }
        let p = reading.expect("读不到处理器性能计数器");
        assert!(
            p.performance_pct > 0.0 && p.performance_pct < 400.0,
            "{p:?}"
        );
        assert!(p.limit_pct > 0.0 && p.limit_pct <= 100.0, "{p:?}");
        // 隔得太近：不再让它读，沿用最近的读数。
        assert_eq!(meter.sample_at(READ_EVERY_MS - 1), Some(p));
    }
}
