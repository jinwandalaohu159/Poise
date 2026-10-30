//! 记录系统的输入：调度每轮交过来的摘要。只有记录需要的几样，与调度的内部结构无关——调度怎么改，
//! 只要还能填出这份摘要，记录系统就不用动。

use poise_core::model::ProcessKey;
use poise_core::prefs::Mode;

/// 一轮调度的摘要。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Round {
    pub unix_ms: u64,
    /// 调度是否还在保持重点程序（重点程序刚结束、两局游戏之间也算）。
    pub focused: bool,
    /// 这一轮的重点程序。
    pub focus: Vec<FocusProgram>,
    /// 这一轮被挪到小核、给重点程序让出大核的程序个数。
    pub moved_aside: u32,
    /// 这一轮在让路（降优先级或让到小核）的后台程序。
    pub yielded: Vec<ProcessKey>,
    pub mode: Mode,
    /// 整机处理器占用（0–100）。
    pub cpu_pct: f32,
    pub cpu_temp: Option<f32>,
    /// 处理器正被固件限速。
    pub throttled: bool,
}

/// 一个重点程序（按主进程）。
#[derive(Debug, Clone, PartialEq)]
pub struct FocusProgram {
    pub key: ProcessKey,
    pub name: String,
    /// 小写完整路径；读不到时为空。
    pub path: Option<String>,
    /// 按游戏调度。
    pub game: bool,
    /// 负载（单核百分比）。
    pub load_pct: u32,
    /// 用到的核心域（位掩码，第 n 位是第 n 个域）。
    pub domains: u16,
}
