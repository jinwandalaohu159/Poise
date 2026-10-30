//! 使用记录：每一段有重点程序的时间（玩游戏、编译、渲染……）一条，由记录系统（poise-logbook）汇总。

use serde::{Deserialize, Serialize};

use poise_core::prefs::Mode;

/// 一段有重点程序的时间。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub started_unix_ms: u64,
    pub ended_unix_ms: u64,
    /// 这段时间里的重点程序，作为重点程序的时间长的在前。
    pub programs: Vec<RecordProgram>,
    /// 用过的模式，按先后，不重复。
    pub modes: Vec<Mode>,
    /// 同一时刻最多有几个没在用的程序被挪到小核（给重点程序让出大核）。
    pub moved_aside_peak: u32,
    /// 期间让过路（降优先级或让到小核）的后台程序个数，按程序计。
    pub yielded_programs: u32,
    /// 整机处理器占用（0–100）的平均与最高。
    pub cpu_pct_avg: f32,
    pub cpu_pct_peak: f32,
    /// 处理器温度的平均与最高；读不到时为空。
    pub cpu_temp_avg: Option<f32>,
    pub cpu_temp_peak: Option<f32>,
    /// 处理器被限速（散热或功耗到了上限）的累计秒数。
    pub throttled_secs: u32,
}

/// 记录里的一个重点程序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordProgram {
    pub name: String,
    /// 小写完整路径（界面取图标用）；读不到时为空。
    pub path: Option<String>,
    /// 是按游戏调度的。
    pub game: bool,
    /// 作为重点程序的累计秒数。
    pub focused_secs: u32,
    /// 最高负载（单核百分比，200 = 两个核心跑满）。
    pub peak_load_pct: u32,
    /// 用过的最多的核心域（位掩码，第 n 位是第 n 个域）。
    pub widest_domains: u16,
}
