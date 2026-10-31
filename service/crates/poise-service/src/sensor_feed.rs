//! 采集线程：定期读取硬件传感器，把最近一次的读数放在共享位置。
//!
//! 机型接口（WMI）一次读取可能要几十到几百毫秒。放在独立线程里，调度引擎和界面请求
//! 只取"最近一次"的结果，永远不用等它。

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::model::Millis;
use poise_hardware::{HardwareReading, SensorLevel, Sensors};

/// 一次读取超过这个时间就记一条日志。
const SLOW_READ: Duration = Duration::from_millis(500);

/// 界面没显示时的读取间隔。机型接口（BIOS 的 WMI 方法）的调用可能让所有核心短暂停顿，系统服务还要
/// 为它花掉不少处理器时间；没人看的时候少读：温度只用于健康状态和走势（是否被限速由处理器自己报告），
/// 30 秒一次足够。界面显示时按 `every` 读。
const QUIET_EVERY: Duration = Duration::from_secs(30);
/// 没人在用（用户离开、Poise 暂停，也没在玩游戏、界面没显示）时的读取间隔：温度只用于健康状态，
/// 过热提示晚几分钟无妨，不为它一夜调用几千次机型接口。
const IDLE_EVERY: Duration = Duration::from_secs(5 * 60);

enum Signal {
    /// 需要的详细程度变了（界面打开、开始玩游戏）：按新的要求立即读一次。
    Level(SensorLevel),
    /// 有没有人在用变了：没人用时读得很少（见 [`IDLE_EVERY`]）；有人回来时立即读一次。
    Idle(bool),
    Stop,
}

/// 采集线程的句柄。丢弃时线程结束。
pub struct SensorFeed {
    latest: Arc<Mutex<HardwareReading>>,
    tx: Sender<Signal>,
    level: SensorLevel,
    idle: bool,
}

impl SensorFeed {
    /// 启动采集线程：界面显示时每隔 `every` 读取一次，否则见 [`QUIET_EVERY`]。
    pub fn start(mut sensors: Sensors, every: Duration) -> Self {
        let latest = Arc::new(Mutex::new(HardwareReading::default()));
        let (tx, rx) = mpsc::channel();
        let shared = latest.clone();
        let started = Instant::now();
        std::thread::Builder::new()
            .name("sensors".into())
            .spawn(move || {
                let mut level = SensorLevel::Basic;
                let mut idle = false;
                let mut read_now = true;
                loop {
                    if read_now {
                        let t = Instant::now();
                        let reading = sensors.read(level, started.elapsed().as_millis() as Millis);
                        if t.elapsed() > SLOW_READ {
                            log::info!("读取硬件传感器耗时 {} ms", t.elapsed().as_millis());
                        }
                        if let Ok(mut l) = shared.lock() {
                            *l = reading;
                        }
                    }
                    let wait = if level.shown() {
                        every
                    } else if idle {
                        IDLE_EVERY
                    } else {
                        QUIET_EVERY
                    };
                    read_now = match rx.recv_timeout(wait) {
                        Ok(Signal::Level(l)) => {
                            level = l;
                            true
                        }
                        Ok(Signal::Idle(now_idle)) => {
                            idle = now_idle;
                            !idle
                        }
                        Err(RecvTimeoutError::Timeout) => true,
                        Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    };
                }
            })
            .expect("创建采集线程失败");
        Self {
            latest,
            tx,
            level: SensorLevel::Basic,
            idle: false,
        }
    }

    /// 最近一次的读数（不等待）。
    pub fn latest(&self) -> HardwareReading {
        self.latest.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// 需要的详细程度。变化时采集线程立即按新的要求读一次。
    pub fn set_level(&mut self, level: SensorLevel) {
        if level != self.level {
            self.level = level;
            let _ = self.tx.send(Signal::Level(level));
        }
    }

    /// 有没有人在用（见 [`IDLE_EVERY`]）。
    pub fn set_idle(&mut self, idle: bool) {
        if idle != self.idle {
            self.idle = idle;
            let _ = self.tx.send(Signal::Idle(idle));
        }
    }
}

impl Drop for SensorFeed {
    fn drop(&mut self) {
        let _ = self.tx.send(Signal::Stop);
    }
}
