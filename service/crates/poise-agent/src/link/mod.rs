//! 与服务的连接。所有管道通信和较慢的采集（音频会话、窗口）都在这个后台线程里做，
//! 托盘线程只发送任务、读取最近一次的状态，永远不会被卡住。
//!
//! 这个线程平时和 Poise 其余部分一样低调（低优先级、只在小核、降频）；只有前台切换时临时提高，
//! 让服务尽快知道（见 [`Worker::foreground_changed`]）。多久采集、保活、重连见 [`cadence`]。
//!
//! 一键清理要等清理做完（重启服务可能十几秒），走单独的线程和连接（见 [`Link::clean_now`]）：
//! 占着这条的话，这期间会话状态报不上去，服务 20 秒收不到上报就会当作 Poise 已经退出。

mod cadence;
mod pipe;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::diagnose::{HealthCode, Level};
use poise_core::model::{Pid, SessionState};
use poise_core::prefs::Mode;
use poise_platform::act::self_tune;
use poise_protocol::Request;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::session::audio::AudioSessions;
use crate::session::{self, Capturer};
use crate::tray::status::TrayStatus;
use pipe::{CallError, PipeClient};

/// 前台切换后等这么久再采集：切换过程中前台会短暂经过别的窗口（Alt+Tab、过渡窗口），
/// 等它稳定下来，一次切换只报一次。
const FOREGROUND_SETTLE: Duration = Duration::from_millis(120);

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
        let worker = Worker {
            rx,
            latest: latest.clone(),
            notify: Box::new(notify),
            client: None,
            reconnect: cadence::Reconnect::default(),
            audio: Vec::new(),
            idle_ms: 0,
            capturer: Capturer::default(),
            last_report: None,
            shown: None,
        };
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

/// 托盘图标显示的：健康状态、需要留意的程度、模式；没连上服务时为 None。
type Shown = Option<(HealthCode, Level, Mode)>;

struct Worker {
    rx: Receiver<Job>,
    latest: Arc<Mutex<Option<TrayStatus>>>,
    notify: Box<dyn Fn() + Send>,
    client: Option<PipeClient>,
    reconnect: cadence::Reconnect,
    audio: Vec<Pid>,
    /// 最近一次采集到的"多久没操作"。
    idle_ms: u64,
    capturer: Capturer,
    /// 最近一次成功上报的状态和时间。
    last_report: Option<(SessionState, Instant)>,
    /// 上次通知托盘时显示的（第一次之前为 None）。
    shown: Option<Shown>,
}

impl Worker {
    fn run(mut self) {
        // 音频会话枚举使用 COM。
        // SAFETY: 本线程加入多线程套间，线程结束前一直有效。
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // COM 对象只能在这个线程里用，所以在这里创建，而不是放在随线程移动的 Worker 里。
        let mut audio = AudioSessions::default();
        let mut next_heartbeat = Instant::now();
        let mut beats = 0u32;
        loop {
            let now = Instant::now();
            if now >= next_heartbeat {
                // 先连上再采集：服务没在运行时不白白枚举窗口和音频。
                if self.ensure_connected().is_ok() {
                    self.audio = audio.active_pids();
                    self.report();
                }
                if beats.is_multiple_of(cadence::STATUS_EVERY_BEATS) {
                    self.refresh_status();
                }
                beats = beats.wrapping_add(1);
                next_heartbeat = now + cadence::heartbeat(self.idle_ms);
            }
            let wait = next_heartbeat.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(wait) {
                Ok(job) => {
                    if self.handle(job) {
                        self.refresh_status();
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
                self.foreground_changed();
                false
            }
            Job::Refresh => {
                self.last_report = None;
                self.report();
                true
            }
            Job::SetMode(mode) => self.fire(Request::SetMode { mode }),
            Job::Boost { pid, minutes } => self.fire(Request::TempBoost { pid, minutes }),
        }
    }

    /// 前台切换：等它稳定一下，把排着的切换一起处理掉，然后立即上报。这一下临时提高线程优先级——
    /// 小核被让路的程序挤满时，也不能排在它们后面。
    fn foreground_changed(&mut self) {
        std::thread::sleep(FOREGROUND_SETTLE);
        let mut later = Vec::new();
        for job in self.rx.try_iter() {
            if !matches!(job, Job::ForegroundChanged) {
                later.push(job);
            }
        }
        self_tune::responsive_thread();
        self.report();
        self_tune::relaxed_thread();
        for job in later {
            if self.handle(job) {
                self.refresh_status();
            }
        }
    }

    fn fire(&mut self, request: Request) -> bool {
        if let Err(e) = self.call::<serde_json::Value>(request) {
            log::info!("请求未完成：{}", e.message());
        }
        true
    }

    /// 上报会话状态（音频沿用心跳里最近一次的结果），没变化时不报。自己的会话不是正在使用的那个
    /// （快速切换用户后切走了、远程连接断开了）时不报：服务只该看正在用电脑的那个人的桌面，
    /// 两个用户的托盘代理都报的话，服务会在两个会话之间来回切换。
    fn report(&mut self) {
        if !session::desktop::session_active() {
            self.last_report = None;
            return;
        }
        let state = self.capturer.capture(&self.audio);
        self.idle_ms = state.idle_ms;
        if let Some((last, _)) = &self.last_report
            && last.fullscreen != state.fullscreen
        {
            log::info!(
                "前台{}全屏（进程 {:?}）",
                if state.fullscreen { "进入" } else { "退出" },
                state.foreground_pid
            );
        }
        let heartbeat = cadence::heartbeat(state.idle_ms);
        let unchanged = self.last_report.as_ref().is_some_and(|(last, at)| {
            !cadence::keepalive_due(at.elapsed(), heartbeat) && session::same_desktop(last, &state)
        });
        if unchanged {
            return;
        }
        let at = Instant::now();
        let sent = self
            .call::<serde_json::Value>(Request::ReportSession(state.clone()))
            .is_ok();
        self.last_report = sent.then_some((state, at));
    }

    /// 取一次状态；托盘要显示的变了才通知托盘线程（它要重读设置、更新图标）。
    fn refresh_status(&mut self) {
        let status = self.call::<TrayStatus>(Request::GetStatus).ok();
        let shown: Shown = status
            .as_ref()
            .map(|s| (s.health.code, s.health.level, s.mode));
        if let Ok(mut s) = self.latest.lock() {
            *s = status;
        }
        if self.shown != Some(shown) {
            self.shown = Some(shown);
            (self.notify)();
        }
    }

    /// 连着服务；没连着时到了重连的时候就连一次（见 [`cadence::Reconnect`]）。
    fn ensure_connected(&mut self) -> Result<&mut PipeClient, CallError> {
        if self.client.is_none() {
            let now = Instant::now();
            if !self.reconnect.due(now) {
                return Err(CallError::Disconnected("等待重连".into()));
            }
            self.reconnect.attempted(now);
            match PipeClient::connect() {
                Ok(client) => {
                    self.reconnect.connected();
                    self.client = Some(client);
                    log::info!("已连接服务");
                }
                Err(e) => {
                    self.reconnect.failed();
                    return Err(e);
                }
            }
        }
        Ok(self.client.as_mut().expect("刚刚已连接"))
    }

    fn call<T: serde::de::DeserializeOwned>(&mut self, request: Request) -> Result<T, CallError> {
        let result = self.ensure_connected()?.call(request);
        if let Err(CallError::Disconnected(e)) = &result {
            log::info!("与服务的连接断开：{e}");
            self.client = None;
        }
        result
    }
}
