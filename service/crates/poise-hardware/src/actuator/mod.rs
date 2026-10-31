//! 硬件执行线程：慢的硬件操作都不在调度线程里做。分成互不等待的两条线：
//!
//! - [`power`]：写入电源设置（电源服务忙时每项几十到几百毫秒）、重新激活电源计划（约 200 毫秒）、
//!   预读电源设置的原值；
//! - [`bios`]：BIOS 切换散热模式（约 1 秒）、风扇加速。
//!
//! 以前两者排在同一个线程里：切换模式时电源设置要等 BIOS 切完才写，第一次接管的预读也要等。
//! 调度引擎只发出命令、从不等待（服务停止时等还原完成除外，见 [`Actuator::settle`]）。

mod bios;
mod power;

use std::time::{Duration, Instant};

use poise_core::prefs::Mode;

use crate::profiles::MachineProfile;
use bios::Bios;
pub use power::PowerWriter;

/// 两条执行线程的句柄。丢弃时线程结束。
pub(crate) struct Actuator {
    power: PowerWriter,
    bios: Bios,
}

impl Actuator {
    pub fn start(profile: Box<dyn MachineProfile>) -> Self {
        Self {
            power: PowerWriter::start(),
            bios: Bios::start(profile),
        }
    }

    pub fn power(&self) -> &PowerWriter {
        &self.power
    }

    pub fn thermal(&self, mode: Mode, on_battery: bool) {
        self.bios.thermal(mode, on_battery);
    }

    pub fn fan_boost(&self, percent: u8) {
        self.bios.fan_boost(percent);
    }

    /// 核对散热模式和风扇加速（睡眠醒来后）。
    pub fn recheck_bios(&self) {
        self.bios.recheck();
    }

    /// 取走"切换散热模式失败了"。
    pub fn take_thermal_failure(&self) -> bool {
        self.bios.take_failure()
    }

    /// 回到 Poise 接管之前的散热模式，收回风扇加速（不等待）。
    pub fn restore_bios(&self) {
        self.bios.restore();
    }

    /// 等两条线做完之前交给它们的事：排着的电源设置写完、BIOS 还原完。各自最多等这么久，同时等。
    pub fn settle(&self, power: Duration, bios: Duration) {
        let started = Instant::now();
        let power_done = self.power.sync();
        let bios_done = self.bios.sync();
        let _ = bios_done.recv_timeout(bios);
        let left = power.saturating_sub(started.elapsed());
        let _ = power_done.recv_timeout(left);
    }
}
