//! 采集线程：定期读取硬件传感器，把最近一次的读数放在共享位置。
//!
//! 机型接口（WMI）一次读取可能要几十到几百毫秒。放在独立线程里，调度引擎和界面请求
//! 只取"最近一次"的结果，永远不用等它。

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::model::Millis;
use poise_hardware::{HardwareReading, Sensors};

/// 一次读取超过这个时间就记一条日志。
const SLOW_READ: Duration = Duration::from_millis(500);

enum Signal {
    /// 需要的详细程度变了（界面打开、开始玩游戏）：按新的要求立即读一次。
    Detailed(bool),
    Stop,
}

/// 采集线程的句柄。丢弃时线程结束。
pub struct SensorFeed {
    latest: Arc<Mutex<HardwareReading>>,
    tx: Sender<Signal>,
    detailed: bool,
}

impl SensorFeed {
    /// 启动采集线程，每隔 `every` 读取一次。
    pub fn start(mut sensors: Sensors, every: Duration) -> Self {
        let latest = Arc::new(Mutex::new(HardwareReading::default()));
        let (tx, rx) = mpsc::channel();
        let shared = latest.clone();
        let started = Instant::now();
        std::thread::Builder::new()
            .name("sensors".into())
            .spawn(move || {
                let mut detailed = false;
                loop {
                    let t = Instant::now();
                    let reading = sensors.read(detailed, started.elapsed().as_millis() as Millis);
                    if t.elapsed() > SLOW_READ {
                        log::info!("读取硬件传感器耗时 {} ms", t.elapsed().as_millis());
                    }
                    if let Ok(mut l) = shared.lock() {
                        *l = reading;
                    }
                    match rx.recv_timeout(every) {
                        Ok(Signal::Detailed(d)) => detailed = d,
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("创建采集线程失败");
        Self {
            latest,
            tx,
            detailed: false,
        }
    }

    /// 最近一次的读数（不等待）。
    pub fn latest(&self) -> HardwareReading {
        self.latest.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// 是否需要详细读数（显卡、风扇）。变化时采集线程立即按新的要求读一次。
    pub fn set_detailed(&mut self, detailed: bool) {
        if detailed != self.detailed {
            self.detailed = detailed;
            let _ = self.tx.send(Signal::Detailed(detailed));
        }
    }
}

impl Drop for SensorFeed {
    fn drop(&mut self) {
        let _ = self.tx.send(Signal::Stop);
    }
}
