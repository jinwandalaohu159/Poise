//! 连接服务的后台线程：定时采集会话状态（前台、全屏、音频、多久没操作）并上报、保活，取托盘要显示的
//! 状态，处理托盘发来的任务；断开了就按 [`super::cadence::Reconnect`] 重连。

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::diagnose::{HealthCode, Level};
use poise_core::model::{Pid, SessionState};
use poise_core::prefs::Mode;
use poise_platform::act::self_tune;
use poise_protocol::Request;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use super::Job;
use super::cadence;
use super::pipe::{CallError, PipeClient};
use crate::session::audio_watch::AudioWatch;
use crate::session::{self, Capturer};
use crate::tray::status::TrayStatus;

/// 前台切换后等这么久再采集：切换过程中前台会短暂经过别的窗口（Alt+Tab、过渡窗口），
/// 等它稳定下来，一次切换只报一次。
const FOREGROUND_SETTLE: Duration = Duration::from_millis(120);

/// 托盘图标显示的：健康状态、需要留意的程度、模式；没连上服务时为 None。
type Shown = Option<(HealthCode, Level, Mode)>;

pub(super) struct Worker {
    rx: Receiver<Job>,
    latest: Arc<Mutex<Option<TrayStatus>>>,
    notify: Box<dyn Fn() + Send>,
    client: Option<PipeClient>,
    reconnect: cadence::Reconnect,
    /// 上次连不上的原因：换了原因才再记一条，不每次重连都记。
    connect_error: Option<String>,
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
    pub(super) fn new(
        rx: Receiver<Job>,
        latest: Arc<Mutex<Option<TrayStatus>>>,
        notify: Box<dyn Fn() + Send>,
    ) -> Self {
        Self {
            rx,
            latest,
            notify,
            client: None,
            reconnect: cadence::Reconnect::default(),
            connect_error: None,
            audio: Vec::new(),
            idle_ms: 0,
            capturer: Capturer::default(),
            last_report: None,
            shown: None,
        }
    }

    pub(super) fn run(mut self) {
        // 采集桌面用到的外壳接口可能用 COM。
        // SAFETY: 本线程加入多线程套间，线程结束前一直有效。
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // 和 0.7.0 一样，这个线程一直及时运行：保活上报不能排在被挪到小核的程序后面——晚到 20 秒服务就当
        // 托盘代理退出了，撤销一切。它每 3 秒只用几毫秒。
        self_tune::responsive_thread();
        // 音频查询在它自己的线程里，卡住时不拖住上报（见 `AudioWatch`）。
        let mut audio = AudioWatch::start();
        let mut next_heartbeat = Instant::now();
        let mut beats = 0u32;
        loop {
            let now = Instant::now();
            if now >= next_heartbeat {
                // 这个会话不是正在用的那个（切走了、远程断开了）：不报（见 `report`），也不白白查音频、
                // 取托盘状态（看不见），隔久一点再看；切回来时有会话通知，立即上报。
                let active = session::desktop::session_active();
                if !active {
                    self.last_report = None;
                } else if self.ensure_connected().is_ok() {
                    // 先连上再采集：服务没在运行时不白白枚举窗口和音频。
                    self.audio = audio.active_pids();
                    self.report();
                }
                // 用户离开了（锁着屏）时托盘没人看：不取状态，回来后（解锁时有会话通知）再取。
                if active
                    && self.idle_ms < cadence::AWAY_AFTER_MS
                    && beats.is_multiple_of(cadence::STATUS_EVERY_BEATS)
                {
                    self.refresh_status();
                }
                beats = beats.wrapping_add(1);
                next_heartbeat = now
                    + if active {
                        cadence::heartbeat(self.idle_ms)
                    } else {
                        cadence::INACTIVE_HEARTBEAT
                    };
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
                // 醒来、解锁、切回这个用户：没连着的话立即重连，不等下一次。
                self.reconnect.now();
                self.last_report = None;
                self.report();
                true
            }
            Job::SetMode(mode) => self.fire(Request::SetMode { mode }),
            Job::Boost { pid, minutes } => self.fire(Request::TempBoost { pid, minutes }),
        }
    }

    /// 前台切换：等它稳定一下，把排着的切换一起处理掉，然后立即上报。
    fn foreground_changed(&mut self) {
        std::thread::sleep(FOREGROUND_SETTLE);
        let mut later = Vec::new();
        for job in self.rx.try_iter() {
            if !matches!(job, Job::ForegroundChanged) {
                later.push(job);
            }
        }
        self.report();
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
                    self.client = Some(client);
                    self.connect_error = None;
                    log::info!("已连接服务");
                }
                Err(e) => {
                    if self.connect_error.as_deref() != Some(e.message()) {
                        log::info!("连不上服务：{}", e.message());
                        self.connect_error = Some(e.message().to_owned());
                    }
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
