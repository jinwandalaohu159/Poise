//! 处理器实际的性能水平与限速：性能计数器 Processor Information（任务管理器"速度"的同一来源）。只读。
//!
//! 硬件自主调频的处理器（Intel 6 代起、AMD Zen 2 起）上，`CallNtPowerInformation` 报告的"当前频率"
//! 永远是基础频率，看不出睿频，也看不出降频。这两个计数器才是实际情况：
//! - `% Processor Performance`：相对基础频率的实际性能（睿频时超过 100）；
//! - `% Performance Limit`：固件允许的上限（温度墙、功耗墙限速时低于 100）。
//!
//! 另外按逻辑处理器读实际频率（`Processor Frequency` 是各自的基础频率，乘上各自的 `% Processor Performance`）：
//! 大核、小核的基础频率不同，整机平均看不出正在跑游戏主线程的那个大核被压到了多少——`% Performance Limit`
//! 在不少处理器上（实测 i7-12700H）功耗墙、温度墙时也一直是 100，只能看实际频率。
//!
//! 还读处理器整体的功耗（`Energy Meter` 的 RAPL 读数，Intel 和 AMD 都有；Windows 自带的计数器，没有的机器
//! 读不到）。所有核心共用一份功耗上限，笔记本平时就贴着它：只靠一两个主线程的游戏帧数上不去时，看它就知道
//! 是功耗被用完了，而不是核心不够。
//!
//! 每次读取得到与上一次之间的平均值，第一次没有可比较的数据。一次读取要 0.5～1.5 毫秒处理器时间
//! （要收集所有逻辑处理器的数据），所以至少隔 [`READ_EVERY_MS`] 才读一次——给的是平均，读得少不丢信息。
//! 系统忙的时候一次读取实测要两三百毫秒，所以在自己的线程里读：采样只取最近一次的读数，从不等待。

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use poise_core::model::Millis;

use crate::win::pdh::{self, Collected, Counter};

const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const LIMIT: &str = r"\Processor Information(_Total)\% Performance Limit";
const CORE_PERFORMANCE: &str = r"\Processor Information(*)\% Processor Performance";
const CORE_BASE_MHZ: &str = r"\Processor Information(*)\Processor Frequency";
/// 各个 RAPL 范围的功率（毫瓦）：整个处理器（实例名带 PKG）、核心、核显、内存。它是按累计能量换算的速率
/// （原始值就是累计能量，实测），读到的是两次采集之间的平均，不是那一刻的抽样。
const ENERGY_POWER: &str = r"\Energy Meter(*)\Power";

/// 两次读取至少隔这么久（采样间隔为 2 秒时隔一轮读一次）。
pub const READ_EVERY_MS: Millis = 3_900;

/// 一次读数（百分比）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CpuPerformance {
    /// 相对基础频率的实际性能。
    pub performance_pct: f32,
    /// 固件允许的性能上限；100 表示没有限速。
    pub limit_pct: f32,
    /// 第 0 组各逻辑处理器运行时的实际频率（MHz），按逻辑处理器编号；这段时间没运行过的是 0，读不到时为空。
    pub core_mhz: Vec<u32>,
    /// 整个处理器这段时间的平均功耗（瓦）；机器不提供时为 None。
    pub package_watts: Option<f32>,
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
                let mut misses = 0u32;
                for () in requests {
                    if query.is_none() && opened_at.is_none_or(|t| t.elapsed() >= REOPEN_EVERY) {
                        opened_at = Some(std::time::Instant::now());
                        query = Query::open();
                    }
                    let reading = query.as_mut().and_then(Query::read);
                    // 打开了却一直读不出（计数器被重建过）：关掉，按上面的节奏重开。
                    misses = if reading.is_some() { 0 } else { misses + 1 };
                    if misses >= REOPEN_AFTER_MISSES {
                        query = None;
                        misses = 0;
                    }
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
        self.latest.lock().ok().and_then(|l| l.clone())
    }
}

struct Query {
    query: pdh::Query,
    performance: Counter,
    limit: Counter,
    core_performance: Counter,
    core_base_mhz: Counter,
    /// 机器不提供功耗计数器时为 None（其余照样读）。
    energy_power: Option<Counter>,
    /// 上次采集的时间。处理器性能是两次采集之间的平均值，隔得太近（刚打开就读）算出来不准。
    collected_at: std::time::Instant,
}

/// 两次采集至少隔这么久。
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
/// 打不开计数器时，多久再试一次。
const REOPEN_EVERY: std::time::Duration = std::time::Duration::from_secs(60);
/// 连续这么多次读不出就关掉重开。
const REOPEN_AFTER_MISSES: u32 = 10;

impl Query {
    fn open() -> Option<Self> {
        let query = pdh::Query::open()?;
        let query = Self {
            performance: query.add(PERFORMANCE)?,
            limit: query.add(LIMIT)?,
            core_performance: query.add(CORE_PERFORMANCE)?,
            core_base_mhz: query.add(CORE_BASE_MHZ)?,
            energy_power: query.add(ENERGY_POWER),
            query,
            collected_at: std::time::Instant::now(),
        };
        query.query.collect();
        Some(query)
    }

    fn read(&mut self) -> Option<CpuPerformance> {
        // 在读取线程里，等一下不影响别人。
        std::thread::sleep(MIN_INTERVAL.saturating_sub(self.collected_at.elapsed()));
        self.collected_at = std::time::Instant::now();
        if self.query.collect() != Collected::Data {
            return None;
        }
        Some(CpuPerformance {
            performance_pct: self.performance.value()? as f32,
            limit_pct: self.limit.value()? as f32,
            core_mhz: core_mhz(
                &per_core(self.core_performance),
                &per_core(self.core_base_mhz),
            ),
            package_watts: self
                .energy_power
                .and_then(Counter::instances)
                .and_then(|power| package_watts(&power)),
        })
    }
}

/// 第 0 组各逻辑处理器的值，按编号（实例名 "0,3" 是第 0 组的 3 号）；读不到时为空。
fn per_core(counter: Counter) -> Vec<(u32, f64)> {
    let mut out: Vec<(u32, f64)> = counter
        .instances()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, v)| Some((name.strip_prefix("0,")?.parse().ok()?, v)))
        .collect();
    out.sort_unstable_by_key(|(i, _)| *i);
    out
}

/// 整个处理器的功耗（瓦）：各个处理器插槽的 PKG 范围加起来（计数器是毫瓦）；没有时为 None。处理器不会
/// 一点电都不用：是 0 说明两次读取隔得太近、计数器还没更新，也当作没读到。
fn package_watts(power_mw: &[(String, f64)]) -> Option<f32> {
    let total: f64 = power_mw
        .iter()
        .filter(|(name, _)| name.to_ascii_uppercase().ends_with("_PKG"))
        .map(|(_, mw)| *mw)
        .sum();
    (total > 0.0).then(|| (total / 1000.0) as f32)
}

/// 各逻辑处理器的实际频率：基础频率 × 相对基础频率的性能。编号不连续、两者对不上时为空。
fn core_mhz(performance: &[(u32, f64)], base_mhz: &[(u32, f64)]) -> Vec<u32> {
    let contiguous = performance
        .iter()
        .enumerate()
        .all(|(k, (i, _))| *i as usize == k);
    if !contiguous || performance.len() != base_mhz.len() {
        return Vec::new();
    }
    performance
        .iter()
        .zip(base_mhz)
        .map(|((_, pct), (_, base))| (base * pct / 100.0).round().max(0.0) as u32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_power_adds_up_the_packages_only() {
        let named = |n: &str, mw: f64| (n.to_string(), mw);
        let one = [
            named("RAPL_Package0_PKG", 41_200.0),
            named("RAPL_Package0_PP0", 32_000.0),
            named("RAPL_Package0_DRAM", 0.0),
        ];
        assert_eq!(package_watts(&one), Some(41.2));
        let two = [
            named("RAPL_Package0_PKG", 100_000.0),
            named("RAPL_Package1_PKG", 50_000.0),
        ];
        assert_eq!(package_watts(&two), Some(150.0));
        assert_eq!(package_watts(&[named("_Total", 0.0)]), None);
        assert_eq!(package_watts(&[named("RAPL_Package0_PKG", 0.0)]), None);
    }

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
        // 刚打开就读，间隔很短：空闲的核心是 0（这段时间没运行过），忙过一下的可能偏离很多，只看个数。
        assert_eq!(
            p.core_mhz.len(),
            crate::observe::processors::read_processor_times().len(),
            "{:?}",
            p.core_mhz
        );
        // 功耗：Intel、AMD 的笔记本和台式机一般都有；有的话不会是 0，也不会离谱。
        if let Some(w) = p.package_watts {
            assert!(w > 0.0 && w < 1000.0, "{p:?}");
        }
        // 隔得太近：不再让它读，沿用最近的读数。
        assert_eq!(meter.sample_at(READ_EVERY_MS - 1), Some(p));
    }
}
