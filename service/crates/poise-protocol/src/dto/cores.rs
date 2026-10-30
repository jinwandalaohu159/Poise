//! 核心与缓存面板：处理器的核心域（大核、按缓存分组的小核、各个 CCD）、各自的缓存与负载，
//! 以及 Poise 此刻把哪些程序放在哪里。

use serde::{Deserialize, Serialize};

use poise_core::model::Pid;
use poise_core::schedule::{Pressure, Verdict};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreMap {
    /// 能把程序限制在一部分核心上（至少两个核心域）。不能时 Poise 不限制核心，只调整优先级。
    pub partitioned: bool,
    /// 智能调度开着（关着时不安排任何程序）。
    pub smart_schedule: bool,
    /// 此刻对处理器的需求（决定后台让路到什么程度）。
    pub demand: Pressure,
    /// 按适合跑重点程序的程度排序：第一个是主域（大核，或缓存最大的 CCD）。
    pub domains: Vec<CoreDomainDto>,
    /// 没有限制核心的程序里负载最高的几个：由 Windows 安排在哪里运行，可能在任何核心上。
    pub unrestricted: Vec<PlacedProgram>,
}

/// 一个核心域：效率等级相同、共用一块缓存的核心。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreDomainDto {
    /// 大核（不是大小核的处理器全部算大核）。
    pub performance: bool,
    pub cores: u32,
    pub threads: u32,
    /// 二级缓存每块的容量，以及每块由几个逻辑处理器共用；读不到时为 0。
    pub l2_bytes: u32,
    pub l2_threads: u32,
    /// 所在三级缓存的容量（读不到时为 0），以及它是第几块：编号相同的核心域共用同一块三级缓存。
    pub l3_bytes: u32,
    pub l3_group: Option<u32>,
    /// 这个域的平均占用（0–100）；还没有读数时为空。
    pub load_pct: Option<f32>,
    /// 各逻辑处理器，按编号。
    pub processors: Vec<ProcessorDto>,
    /// Poise 放在这里的程序：重点程序在前，其次是让路的、挪开的；同类里负载高的在前。
    pub programs: Vec<PlacedProgram>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProcessorDto {
    /// 逻辑处理器编号（任务管理器里的 CPU 编号）。
    pub number: u32,
    /// 属于哪个物理核心：编号相同的逻辑处理器是同一个核心的两个线程（超线程）。
    pub core: u32,
    /// 占用（0–100）；读不到时为空。
    pub load_pct: Option<f32>,
}

/// 被安排在某个核心域上的程序（限制了核心的），或者由 Windows 安排、哪里都能跑的程序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedProgram {
    pub pid: Pid,
    pub name: String,
    /// 小写完整路径（界面取图标用）；读不到时为空。
    pub path: String,
    /// 为什么在这里：按游戏调度、重点程序、让路、移到小核……
    pub verdict: Verdict,
    /// 负载（单核百分比，200 = 两个核心跑满）。
    pub load_pct: u32,
    /// 它同时用着几个核心域（重点程序需要更多线程时会逐个加上）。
    pub domains: u32,
}
