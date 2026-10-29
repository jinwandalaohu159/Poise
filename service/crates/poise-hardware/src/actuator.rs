//! 硬件执行线程：慢的硬件操作——重新激活电源计划（约 200 毫秒）、BIOS 切换散热模式（约 1 秒）、
//! 风扇加速——都在这个线程里执行。调度引擎只发出命令、从不等待，前台切换和界面请求不会被它卡住。
//!
//! 连续到达的命令合并后只执行最新的（快速切换模式时，中间的模式不会真的落到硬件上），
//! 顺序固定为：电源计划 → 散热模式 → 风扇加速（BIOS 切换散热模式时会把风扇加速清零，必须在它之后写）。

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use poise_core::prefs::Mode;

use crate::profiles::MachineProfile;

enum Command {
    /// 重新激活电源计划，让已经写入的设置生效。
    ActivatePower,
    Thermal {
        mode: Mode,
        on_battery: bool,
    },
    FanBoost(u8),
    /// 回到 Poise 接管之前的散热模式并关闭风扇加速，完成后回复。
    Restore {
        boost_on: bool,
        done: Sender<()>,
    },
}

/// 执行线程的句柄。丢弃时线程结束。
pub(crate) struct Actuator {
    tx: Sender<Command>,
}

impl Actuator {
    pub fn start(profile: Box<dyn MachineProfile>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hardware".into())
            .spawn(move || run(profile, rx))
            .expect("创建硬件执行线程失败");
        Self { tx }
    }

    pub fn activate_power(&self) {
        let _ = self.tx.send(Command::ActivatePower);
    }

    pub fn thermal(&self, mode: Mode, on_battery: bool) {
        let _ = self.tx.send(Command::Thermal { mode, on_battery });
    }

    pub fn fan_boost(&self, percent: u8) {
        let _ = self.tx.send(Command::FanBoost(percent));
    }

    /// 还原散热模式与风扇加速，等它完成（最多 `timeout`）。服务停止、Poise 退出时调用。
    pub fn restore(&self, boost_on: bool, timeout: Duration) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(Command::Restore { boost_on, done }).is_ok() {
            let _ = wait.recv_timeout(timeout);
        }
    }
}

/// 一批合并后的待办。
#[derive(Default)]
struct Pending {
    activate: bool,
    thermal: Option<(Mode, bool)>,
    boost: Option<u8>,
    restore: Option<(bool, Sender<()>)>,
}

impl Pending {
    fn add(&mut self, command: Command) {
        match command {
            Command::ActivatePower => self.activate = true,
            Command::Thermal { mode, on_battery } => self.thermal = Some((mode, on_battery)),
            Command::FanBoost(p) => self.boost = Some(p),
            Command::Restore { boost_on, done } => self.restore = Some((boost_on, done)),
        }
    }
}

fn run(mut profile: Box<dyn MachineProfile>, rx: Receiver<Command>) {
    while let Ok(first) = rx.recv() {
        let mut pending = Pending::default();
        pending.add(first);
        while let Ok(more) = rx.try_recv() {
            pending.add(more);
        }
        if pending.activate
            && let Err(e) = poise_platform::settings::power_plan::activate()
        {
            log::warn!("重新激活电源计划失败：{e:?}");
        }
        if let Some((boost_on, done)) = pending.restore {
            // 要还原了：之前排队的模式和风扇加速不必再执行。
            profile.restore_thermal_mode();
            if boost_on && let Err(e) = profile.set_fan_boost(0) {
                log::warn!("关闭风扇加速失败：{e}");
            }
            let _ = done.send(());
            continue;
        }
        if let Some((mode, on_battery)) = pending.thermal
            && let Err(e) = profile.apply_thermal_mode(mode, on_battery)
        {
            log::warn!("切换散热模式失败：{e}");
        }
        if let Some(percent) = pending.boost
            && let Err(e) = profile.set_fan_boost(percent)
        {
            log::warn!("风扇加速失败：{e}");
        }
    }
}
