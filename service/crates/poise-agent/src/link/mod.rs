//! 与服务的连接。所有管道通信和较慢的采集（音频会话、窗口）都在这个后台线程里做，
//! 托盘线程只发送任务、读取最近一次的状态，永远不会被卡住。
//!
//! 这个线程一直及时运行（不受 Poise 进程的低优先级、只在小核、降频影响）：前台切换要让服务尽快知道，
//! 保活上报也不能晚到。线程里做什么见 `worker`，多久采集、保活、重连见 [`cadence`]。
//!
//! 一键清理要等清理做完（重启服务可能十几秒），走单独的线程和连接（见 [`Link::clean_now`]）：
//! 占着这条的话，这期间会话状态报不上去，服务 20 秒收不到上报就会当作 Poise 已经退出。

mod cadence;
mod pipe;
mod worker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use poise_core::model::Pid;
use poise_core::prefs::Mode;
use poise_protocol::Request;

use crate::tray::status::TrayStatus;
use pipe::PipeClient;
use worker::Worker;

pub enum Job {
    /// 前台窗口变了：立即上报（不重新枚举音频）。
    ForegroundChanged,
    /// 刚从睡眠里醒来、会话切换了、一键清理做完了：不管有没有变化都上报一次，刷新状态。
    Refresh,
    SetMode(Mode),
    Boost {
        pid: Pid,
        minutes: u32,
    },
}

pub struct Link {
    tx: Sender<Job>,
    /// 最近一次从服务得到的状态（服务没连上时为 None）。
    latest: Arc<Mutex<Option<TrayStatus>>>,
    /// 一键清理正在进行（同一时间只做一次）。
    cleaning: Arc<AtomicBool>,
}

impl Link {
    /// 启动后台线程。托盘要显示的状态变了时调用 `notify`（在后台线程上调用，应只投递消息）。
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let worker = Worker::new(rx, latest.clone(), Box::new(notify));
        std::thread::Builder::new()
            .name("service-link".into())
            .spawn(move || worker.run())
            .expect("创建后台线程失败");
        Self {
            tx,
            latest,
            cleaning: Arc::default(),
        }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// 一键清理：在单独的线程里、用单独的连接等它做完，做完后刷新状态（见模块说明）。
    pub fn clean_now(&self) {
        if self.cleaning.swap(true, Ordering::AcqRel) {
            return;
        }
        let (cleaning, tx) = (self.cleaning.clone(), self.tx.clone());
        let spawned = std::thread::Builder::new()
            .name("clean-now".into())
            .spawn(move || {
                let done = PipeClient::connect()
                    .and_then(|mut c| c.call::<serde_json::Value>(Request::CleanNow));
                if let Err(e) = done {
                    log::info!("一键清理未完成：{}", e.message());
                }
                cleaning.store(false, Ordering::Release);
                let _ = tx.send(Job::Refresh);
            });
        if spawned.is_err() {
            self.cleaning.store(false, Ordering::Release);
        }
    }

    /// 最近一次从服务得到的状态（服务没连上时为 None）。
    pub fn status(&self) -> Option<TrayStatus> {
        self.latest.lock().ok().and_then(|s| s.clone())
    }
}
