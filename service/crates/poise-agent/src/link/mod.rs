//! 与服务的连接。所有管道通信和较慢的采集（音频会话）都在这个后台线程里做，
//! 托盘线程只发送任务、读取最近一次的状态，永远不会被卡住。

mod pipe;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::model::{Pid, SessionState};
use poise_core::prefs::Mode;
use poise_protocol::{Request, Status};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::session;
use crate::session::audio::AudioSessions;
use pipe::{CallError, PipeClient};

/// 会话心跳：每 3 秒采集一次（音频会话只在心跳里重新枚举，开始发声的后台程序最迟 3 秒内被照顾到），
/// 有变化才上报；前台变化时立即上报，沿用上次的音频结果，上报更快。
const HEARTBEAT: Duration = Duration::from_secs(3);
/// 状态没变时也至少隔这么久上报一次：服务据此知道托盘代理还在（10 秒没有上报就视为已退出），
/// 也顺便更新"多久没操作"。
const KEEPALIVE: Duration = Duration::from_secs(6);
/// 托盘图标的状态刷新间隔。
const STATUS_EVERY: Duration = Duration::from_secs(10);
/// 服务没在运行时，重新连接的最小间隔。
const RECONNECT_EVERY: Duration = Duration::from_secs(3);

pub enum Job {
    /// 前台窗口变了：立即上报（不重新枚举音频）。
    ForegroundChanged,
    SetMode(Mode),
    CleanNow,
    Boost {
        pid: Pid,
        minutes: u32,
    },
}

pub struct Link {
    tx: Sender<Job>,
    /// 最近一次从服务得到的状态（服务没连上时为 None）。
    latest: Arc<Mutex<Option<Status>>>,
}

impl Link {
    /// 启动后台线程。状态有变化时调用 `notify`（在后台线程上调用，应只投递消息）。
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let worker = Worker {
            rx,
            latest: latest.clone(),
            notify: Box::new(notify),
            client: None,
            last_connect: None,
            audio: Vec::new(),
            last_report: None,
        };
        std::thread::Builder::new()
            .name("service-link".into())
            .spawn(move || {
                // 前台切换的通知经这个线程发给服务。
                poise_platform::act::self_tune::responsive_thread();
                worker.run()
            })
            .expect("创建后台线程失败");
        Self { tx, latest }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// 最近一次从服务得到的状态（服务没连上时为 None）。
    pub fn status(&self) -> Option<Status> {
        self.latest.lock().ok().and_then(|s| s.clone())
    }
}

struct Worker {
    rx: Receiver<Job>,
    latest: Arc<Mutex<Option<Status>>>,
    notify: Box<dyn Fn() + Send>,
    client: Option<PipeClient>,
    last_connect: Option<Instant>,
    audio: Vec<Pid>,
    /// 最近一次成功上报的状态和时间。
    last_report: Option<(SessionState, Instant)>,
}

impl Worker {
    fn run(mut self) {
        // 音频会话枚举使用 COM。
        // SAFETY: 本线程加入多线程套间，线程结束前一直有效。
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // COM 对象只能在这个线程里用，所以在这里创建，而不是放在随线程移动的 Worker 里。
        let mut audio = AudioSessions::default();
        let mut next_heartbeat = Instant::now();
        let mut next_status = Instant::now();
        loop {
            let now = Instant::now();
            if now >= next_heartbeat {
                self.audio = audio.active_pids();
                self.report();
                next_heartbeat = now + HEARTBEAT;
            }
            if now >= next_status {
                self.refresh_status();
                next_status = now + STATUS_EVERY;
            }
            let wait = next_heartbeat
                .min(next_status)
                .saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(wait) {
                Ok(job) => {
                    if self.handle(job) {
                        self.refresh_status();
                        next_status = Instant::now() + STATUS_EVERY;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// 处理任务，返回之后是否需要立即刷新状态。
    fn handle(&mut self, job: Job) -> bool {
        match job {
            Job::ForegroundChanged => {
                self.report();
                false
            }
            Job::SetMode(mode) => self.fire(Request::SetMode { mode }),
            Job::CleanNow => self.fire(Request::CleanNow),
            Job::Boost { pid, minutes } => self.fire(Request::TempBoost { pid, minutes }),
        }
    }

    fn fire(&mut self, request: Request) -> bool {
        if let Err(e) = self.call::<serde_json::Value>(request) {
            log::info!("请求未完成：{}", e.message());
        }
        true
    }

    /// 上报会话状态（音频沿用心跳里最近一次的结果），没变化时不报。
    fn report(&mut self) {
        let state = session::capture(&self.audio);
        let unchanged = self.last_report.as_ref().is_some_and(|(last, at)| {
            at.elapsed() < KEEPALIVE && session::same_desktop(last, &state)
        });
        if unchanged {
            return;
        }
        let sent = self
            .call::<serde_json::Value>(Request::ReportSession(state.clone()))
            .is_ok();
        self.last_report = sent.then(|| (state, Instant::now()));
    }

    fn refresh_status(&mut self) {
        let status = self.call::<Status>(Request::GetStatus).ok();
        if let Ok(mut s) = self.latest.lock() {
            *s = status;
        }
        (self.notify)();
    }

    fn call<T: serde::de::DeserializeOwned>(&mut self, request: Request) -> Result<T, CallError> {
        if self.client.is_none() {
            let throttled = self
                .last_connect
                .is_some_and(|t| t.elapsed() < RECONNECT_EVERY);
            if throttled {
                return Err(CallError::Disconnected("等待重连".into()));
            }
            self.last_connect = Some(Instant::now());
            self.client = Some(PipeClient::connect()?);
            log::info!("已连接服务");
        }
        let client = self.client.as_mut().expect("刚刚已连接");
        let result = client.call(request);
        if let Err(CallError::Disconnected(e)) = &result {
            log::info!("与服务的连接断开：{e}");
            self.client = None;
        }
        result
    }
}
