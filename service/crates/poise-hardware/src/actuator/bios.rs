//! BIOS 执行线程：切换散热模式（机型接口，约 1 秒）、风扇加速。
//!
//! 连续到达的命令只执行最新的（快速切换模式时，中间的散热模式不会真的落到硬件上）；还原取消之前排着的，
//! 之后来的（Poise 退出后马上又打开）照常执行。顺序固定为：还原 → 散热模式 → 风扇加速。
//!
//! 风扇加速 0 表示不管：BIOS 按温度调节，用户在 AWCC 里设的照旧。Poise 写过非 0 的加速之后才会写 0 收回；
//! BIOS 切换散热模式时会把风扇加速清零（G15 实测），之后重新写入正在用的加速。

use std::sync::mpsc::{self, Receiver, Sender};

use poise_core::prefs::Mode;

use crate::profiles::MachineProfile;

enum Command {
    Thermal {
        mode: Mode,
        on_battery: bool,
    },
    FanBoost(u8),
    /// 回到 Poise 接管之前的散热模式，收回风扇加速。
    Restore,
    /// 之前发出的命令都执行完后回复。
    Sync(Sender<()>),
}

pub(super) struct Bios {
    tx: Sender<Command>,
}

impl Bios {
    pub fn start(profile: Box<dyn MachineProfile>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("bios".into())
            .spawn(move || {
                let mut worker = Worker { profile, boost: 0 };
                while let Ok(first) = rx.recv() {
                    let mut pending = Pending::default();
                    pending.add(first);
                    while let Ok(more) = rx.try_recv() {
                        pending.add(more);
                    }
                    worker.run(pending);
                }
            })
            .expect("创建 BIOS 执行线程失败");
        Self { tx }
    }

    pub fn thermal(&self, mode: Mode, on_battery: bool) {
        let _ = self.tx.send(Command::Thermal { mode, on_battery });
    }

    pub fn fan_boost(&self, percent: u8) {
        let _ = self.tx.send(Command::FanBoost(percent));
    }

    pub fn restore(&self) {
        let _ = self.tx.send(Command::Restore);
    }

    /// 之前发出的命令都执行完时回复。
    pub fn sync(&self) -> Receiver<()> {
        let (done, finished) = mpsc::channel();
        let _ = self.tx.send(Command::Sync(done));
        finished
    }
}

/// 一批合并后的待办。
#[derive(Default)]
struct Pending {
    restore: bool,
    thermal: Option<(Mode, bool)>,
    boost: Option<u8>,
    synced: Vec<Sender<()>>,
}

impl Pending {
    fn add(&mut self, command: Command) {
        match command {
            Command::Thermal { mode, on_battery } => self.thermal = Some((mode, on_battery)),
            Command::FanBoost(p) => self.boost = Some(p),
            // 要还原了：之前排着的模式和风扇加速不必再执行；之后来的照常执行。
            Command::Restore => {
                self.thermal = None;
                self.boost = None;
                self.restore = true;
            }
            Command::Sync(done) => self.synced.push(done),
        }
    }
}

struct Worker {
    profile: Box<dyn MachineProfile>,
    /// Poise 写入、此刻在用的风扇加速；0 表示没有加速（不管）。
    boost: u8,
}

impl Worker {
    fn run(&mut self, pending: Pending) {
        if pending.restore {
            self.profile.restore_thermal_mode();
            self.set_boost(0, false);
        }
        let mut cleared = false;
        if let Some((mode, on_battery)) = pending.thermal {
            match self.profile.apply_thermal_mode(mode, on_battery) {
                Ok(switched) => cleared = switched,
                Err(e) => log::warn!("切换散热模式失败：{e}"),
            }
        }
        let boost = pending.boost.unwrap_or(self.boost);
        self.set_boost(boost, cleared);
        for done in pending.synced {
            let _ = done.send(());
        }
    }

    /// 写入风扇加速。和在用的相同时不写，除非 BIOS 刚把它清零（`cleared`）；没加速过时不写 0。
    fn set_boost(&mut self, percent: u8, cleared: bool) {
        let unchanged = percent == self.boost && !(cleared && percent > 0);
        if unchanged {
            return;
        }
        match self.profile.set_fan_boost(percent) {
            Ok(()) => self.boost = percent,
            Err(e) => {
                log::warn!("风扇加速失败：{e}");
                // 加速可能写进了一部分风扇：还原时照样收回。
                self.boost = self.boost.max(percent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// 记下每次写入的风扇加速；切换散热模式时按 `switches` 报告是否写了 BIOS。
    struct Recorder {
        boosts: Arc<Mutex<Vec<u8>>>,
        switches: bool,
    }
    impl MachineProfile for Recorder {
        fn name(&self) -> &str {
            "test"
        }
        fn apply_thermal_mode(&mut self, _: Mode, _: bool) -> Result<bool, String> {
            Ok(self.switches)
        }
        fn set_fan_boost(&mut self, percent: u8) -> Result<(), String> {
            self.boosts.lock().unwrap().push(percent);
            Ok(())
        }
    }

    fn worker(switches: bool) -> (Worker, Arc<Mutex<Vec<u8>>>) {
        let boosts = Arc::new(Mutex::new(Vec::new()));
        let profile = Recorder {
            boosts: boosts.clone(),
            switches,
        };
        (
            Worker {
                profile: Box::new(profile),
                boost: 0,
            },
            boosts,
        )
    }

    fn batch(commands: Vec<Command>) -> Pending {
        let mut p = Pending::default();
        for c in commands {
            p.add(c);
        }
        p
    }

    fn thermal() -> Command {
        Command::Thermal {
            mode: Mode::Full,
            on_battery: false,
        }
    }

    #[test]
    fn commands_after_a_restore_still_run_and_earlier_ones_do_not() {
        let p = batch(vec![thermal(), Command::FanBoost(60), Command::Restore]);
        assert_eq!((p.thermal, p.boost), (None, None));
        // Poise 退出后马上又打开：还原之后再切到新的模式。
        let mut p = p;
        p.add(Command::Thermal {
            mode: Mode::Quiet,
            on_battery: false,
        });
        assert_eq!(p.thermal, Some((Mode::Quiet, false)));
        assert!(p.restore);
    }

    #[test]
    fn no_boost_is_hands_off_until_poise_boosts() {
        let (mut w, boosts) = worker(true);
        // 没加速过：切换模式、要求 0，都不写（用户在 AWCC 里设的照旧）。
        w.run(batch(vec![thermal(), Command::FanBoost(0)]));
        w.run(batch(vec![Command::Restore]));
        assert!(boosts.lock().unwrap().is_empty());
        w.run(batch(vec![Command::FanBoost(60)]));
        w.run(batch(vec![Command::Restore]));
        assert_eq!(*boosts.lock().unwrap(), [60, 0]);
    }

    #[test]
    fn boost_is_written_again_only_when_the_bios_cleared_it() {
        let (mut w, boosts) = worker(true);
        w.run(batch(vec![Command::FanBoost(60)]));
        w.run(batch(vec![thermal()]));
        let (mut quiet, untouched) = worker(false);
        quiet.run(batch(vec![Command::FanBoost(60)]));
        quiet.run(batch(vec![thermal(), Command::FanBoost(60)]));
        assert_eq!(*boosts.lock().unwrap(), [60, 60]);
        assert_eq!(*untouched.lock().unwrap(), [60]);
    }
}
