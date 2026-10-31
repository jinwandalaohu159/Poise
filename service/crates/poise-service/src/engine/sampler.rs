//! 进程快照的采集：每轮最费时的一步（几百个进程约 5 毫秒，整机很忙时更久）。放在引擎锁外——
//! 采样线程先取快照、再锁住引擎处理它（见 `runtime`），前台切换和界面的请求不用等采集；
//! 引擎自己临时要一份快照时（刚打开的程序、撤销调整）也经这里。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use poise_core::model::{Millis, Snapshot};
use poise_platform::observe::process::snapshot::Snapshotter;

use super::phases::{self, Spent};

/// 一次采集的结果与花的时间。
pub struct Sampled {
    /// 采集失败时为空（已记日志）。
    pub snapshot: Option<Snapshot>,
    pub spent: Spent,
}

/// 采集进程快照，可在引擎锁外使用。时间与引擎的时间一致（从引擎创建时算起）。
#[derive(Clone)]
pub struct Sampler {
    snapshotter: Arc<Mutex<Snapshotter>>,
    started: Instant,
}

impl Sampler {
    pub(super) fn new(started: Instant) -> Self {
        Self {
            snapshotter: Arc::new(Mutex::new(Snapshotter::start())),
            started,
        }
    }

    fn now(&self) -> Millis {
        self.started.elapsed().as_millis() as Millis
    }

    /// 取一份快照，连同花的时间。
    pub fn sample(&self) -> Sampled {
        let (snapshot, spent) = phases::measure(|| self.take(self.now()));
        Sampled { snapshot, spent }
    }

    /// 取一份快照；失败时记日志，返回空。
    pub(super) fn take(&self, now: Millis) -> Option<Snapshot> {
        let mut snapshotter = self.snapshotter.lock().unwrap_or_else(|e| e.into_inner());
        match snapshotter.take(now) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("采样失败：{e}");
                None
            }
        }
    }
}
