//! 显卡占用测量线程：测量指定进程（前台全屏的那个）的 3D 占用，把最近一次的结果放在共享位置。
//!
//! 平时直接问显卡内核，很快；但受保护的进程只能退回性能计数器，打开一次要几百毫秒（系统要枚举所有
//! 进程的显卡引擎）。所以放在独立线程里，调度引擎只取"最近一次"的结果，前台切换和界面请求不用等它。

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poise_core::model::Pid;
use poise_platform::observe::gpu::GpuMeter;

/// 换了测量的进程后，隔这么久就读第一次（计数器需要两次采样才有读数），不必等一整个测量间隔。
const FIRST_READING_AFTER: Duration = Duration::from_millis(500);

/// 要测量的进程变了（`None` 表示停止测量），或者结束线程。
enum Signal {
    Target(Option<Pid>),
    Stop,
}

/// 测量线程的句柄。丢弃时线程结束。
pub struct GpuFeed {
    latest: Arc<Mutex<Option<(Pid, f32)>>>,
    tx: Sender<Signal>,
    target: Option<Pid>,
}

impl GpuFeed {
    /// 启动测量线程，有目标时每隔 `every` 测一次。
    pub fn start(every: Duration) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        let shared = latest.clone();
        std::thread::Builder::new()
            .name("gpu".into())
            .spawn(move || {
                // 显卡占用用来认游戏：小核被让路的程序挤满时也要按时测到。
                poise_platform::act::self_tune::responsive_thread();
                let mut meter = GpuMeter::default();
                let mut target: Option<Pid> = None;
                let mut switched = false;
                loop {
                    let reading = target.and_then(|pid| meter.usage_3d(pid).map(|pct| (pid, pct)));
                    if let Ok(mut l) = shared.lock() {
                        *l = reading;
                    }
                    let wait = if std::mem::take(&mut switched) {
                        FIRST_READING_AFTER
                    } else {
                        every
                    };
                    let signal = if target.is_some() {
                        rx.recv_timeout(wait)
                    } else {
                        rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                    };
                    match signal {
                        Ok(Signal::Target(t)) => {
                            target = t;
                            switched = t.is_some();
                            if t.is_none() {
                                meter.stop();
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("创建显卡测量线程失败");
        Self {
            latest,
            tx,
            target: None,
        }
    }

    /// 设置要测量的进程。变化时测量线程立即切换。
    pub fn measure(&mut self, target: Option<Pid>) {
        if target != self.target {
            self.target = target;
            let _ = self.tx.send(Signal::Target(target));
        }
    }

    /// 当前目标最近一次的测量结果（不等待）；还没测到时为空。
    pub fn latest(&self) -> Option<(Pid, f32)> {
        let reading = self.latest.lock().ok().and_then(|l| *l)?;
        (Some(reading.0) == self.target).then_some(reading)
    }
}

impl Drop for GpuFeed {
    fn drop(&mut self) {
        let _ = self.tx.send(Signal::Stop);
    }
}
