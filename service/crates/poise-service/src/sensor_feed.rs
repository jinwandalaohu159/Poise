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

enum Signal {
    /// 需要的详细程度变了（界面打开、开始玩游戏）：按新的要求立即读一次。
    Level(SensorLevel),
    Stop,
}

/// 采集线程的句柄。丢弃时线程结束。
pub struct SensorFeed {
    latest: Arc<Mutex<HardwareReading>>,
    tx: Sender<Signal>,
    level: SensorLevel,
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
                loop {
                    let t = Instant::now();
                    let reading = sensors.read(level, started.elapsed().as_millis() as Millis);
                    if t.elapsed() > SLOW_READ {
                        log::info!("读取硬件传感器耗时 {} ms", t.elapsed().as_millis());
                    }
                    if let Ok(mut l) = shared.lock() {
                        *l = reading;
                    }
                    let wait = if level == SensorLevel::Full {
                        every
                    } else {
                        QUIET_EVERY
                    };
                    match rx.recv_timeout(wait) {
                        Ok(Signal::Level(l)) => level = l,
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("创建采集线程失败");
        Self {
            latest,
            tx,
            level: SensorLevel::Basic,
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
}

impl Drop for SensorFeed {
    fn drop(&mut self) {
        let _ = self.tx.send(Signal::Stop);
    }
}
