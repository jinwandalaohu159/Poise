//! 命名管道客户端：一问一答，每条消息一行 JSON。每次调用都有时限（见 [`Watchdog`]）：服务卡住时
//! 不一直等下去——否则托盘会一直显示旧的状态，前台切换也不再上报。
//!
//! 只连真的服务：管道的另一头必须是和托盘代理装在同一目录的程序（别的程序抢先建了同名管道时不连），
//! 而且只允许对方确认身份、不允许以用户的身份做事（`SECURITY_IDENTIFICATION`）。

use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use poise_platform::win::handle::{OwnedHandle, image_path};
use poise_platform::win::wide::WideString;
use poise_protocol::{
    PIPE_NAME, PROTOCOL_VERSION, Request, RequestEnvelope, ResponseEnvelope, read_message,
    write_message,
};
use serde::de::DeserializeOwned;
use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_PIPE_BUSY, HANDLE};
use windows::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
use windows::Win32::System::IO::CancelSynchronousIo;
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};

/// 一般请求最多等这么久。
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// 一键清理要等清理执行完（重启服务最多要等十几秒）。
const CLEAN_TIMEOUT: Duration = Duration::from_secs(60);
/// 管道的实例都在用时（服务正在接受上一个连接），最多等这么久再连一次。
const BUSY_WAIT_MS: u32 = 1_000;
/// 取消读写没赶上（那一刻线程还没进入读写）时，隔这么久再取消一次，直到调用结束。
const CANCEL_RETRY: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub enum CallError {
    /// 连接断开、服务没在运行或没有及时回复。
    Disconnected(String),
    /// 服务拒绝了请求。
    Rejected(String),
}

impl CallError {
    pub fn message(&self) -> &str {
        match self {
            Self::Disconnected(m) | Self::Rejected(m) => m,
        }
    }
}

pub struct PipeClient {
    writer: File,
    reader: BufReader<File>,
    next_id: u64,
    watchdog: Watchdog,
}

impl PipeClient {
    /// 连接服务并握手。必须在之后发请求的那个线程上调用（超时时取消的是这个线程上的读写）。
    pub fn connect() -> Result<Self, CallError> {
        let file = open_pipe().map_err(|e| CallError::Disconnected(e.to_string()))?;
        if !genuine_server(&file) {
            return Err(CallError::Disconnected("管道另一头不是 Poise 服务".into()));
        }
        let reader = BufReader::new(
            file.try_clone()
                .map_err(|e| CallError::Disconnected(e.to_string()))?,
        );
        let mut client = Self {
            writer: file,
            reader,
            next_id: 1,
            watchdog: Watchdog::for_current_thread()
                .ok_or_else(|| CallError::Disconnected("无法创建超时检查".into()))?,
        };
        client.call::<serde_json::Value>(Request::Hello {
            protocol: PROTOCOL_VERSION,
        })?;
        Ok(client)
    }

    pub fn call<T: DeserializeOwned>(&mut self, request: Request) -> Result<T, CallError> {
        let timeout = if matches!(request, Request::CleanNow) {
            CLEAN_TIMEOUT
        } else {
            CALL_TIMEOUT
        };
        self.watchdog.arm(timeout);
        let result = self.exchange(request);
        self.watchdog.disarm();
        result
    }

    fn exchange<T: DeserializeOwned>(&mut self, request: Request) -> Result<T, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let disconnected = |e: poise_protocol::FramingError| CallError::Disconnected(e.to_string());
        write_message(&mut self.writer, &RequestEnvelope { id, request }).map_err(disconnected)?;
        let response: ResponseEnvelope = read_message(&mut self.reader)
            .map_err(disconnected)?
            .ok_or_else(|| CallError::Disconnected("服务关闭了连接".into()))?;
        if response.id != id {
            // 回复和请求对不上（之前某次没读完）：断开重连，不把别的回复当成这次的。
            return Err(CallError::Disconnected("回复与请求不对应".into()));
        }
        if !response.ok {
            return Err(CallError::Rejected(response.error.unwrap_or_default()));
        }
        let result = response.result.as_deref().map_or("null", |r| r.get());
        serde_json::from_str(result).map_err(|e| CallError::Rejected(e.to_string()))
    }
}

/// 打开管道；所有实例都在用时等一会儿再试一次。
fn open_pipe() -> std::io::Result<File> {
    let open = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0)
            .open(PIPE_NAME)
    };
    match open() {
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
            let name = WideString::new(PIPE_NAME);
            // SAFETY: 名称在调用期间有效。
            let _ = unsafe { WaitNamedPipeW(name.as_pcwstr(), BUSY_WAIT_MS) };
            open()
        }
        other => other,
    }
}

/// 管道的另一头是不是和托盘代理装在同一目录的程序（服务）。
fn genuine_server(pipe: &File) -> bool {
    let mut pid = 0u32;
    // SAFETY: 句柄是刚打开的管道，输出到本地变量。
    let ok = unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.is_ok();
    let server_dir = ok
        .then(|| image_path(pid))
        .flatten()
        .and_then(|p| std::path::Path::new(&p).parent().map(|d| d.to_path_buf()));
    let own_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    match (server_dir, own_dir) {
        (Some(server), Some(own)) => server
            .to_string_lossy()
            .eq_ignore_ascii_case(&own.to_string_lossy()),
        _ => false,
    }
}

/// 调用的时限：超时时取消发请求那个线程上正在进行的管道读写，调用随即以"断开"失败，之后重新连接。
///
/// 检查线程平时睡着：没有调用在进行时一直等，有调用时等到它的时限（不轮询，托盘常驻期间不白白醒来）。
/// 取消在锁内进行：调用这时还没结束（结束时要先拿到锁才能解除时限），取消的一定是超时的那次读写，
/// 不会误伤下一次调用。取消的那一刻线程可能正好不在读写里（刚写完、还没开始读），取消落空：
/// 隔一会儿再取消，直到调用结束。连接关闭（这个对象被丢弃）后检查线程结束。
struct Watchdog {
    shared: Arc<(Mutex<Deadline>, Condvar)>,
}

#[derive(Default)]
struct Deadline {
    at: Option<Instant>,
    closed: bool,
}

impl Watchdog {
    fn for_current_thread() -> Option<Self> {
        let thread = current_thread()?;
        let shared = Arc::new((Mutex::new(Deadline::default()), Condvar::new()));
        let watched = shared.clone();
        std::thread::Builder::new()
            .name("service-link-watchdog".into())
            .spawn(move || watch(&watched, &thread))
            .ok()?;
        Some(Self { shared })
    }

    fn set(&self, update: impl FnOnce(&mut Deadline)) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut d) = lock.lock() {
            update(&mut d);
            wake.notify_one();
        }
    }

    fn arm(&self, timeout: Duration) {
        self.set(|d| d.at = Some(Instant::now() + timeout));
    }

    fn disarm(&self) {
        self.set(|d| d.at = None);
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.set(|d| d.closed = true);
    }
}

/// 检查线程：等到时限，到了还没解除就取消那个线程上的读写。
fn watch((lock, wake): &(Mutex<Deadline>, Condvar), thread: &OwnedHandle) {
    let Ok(mut d) = lock.lock() else {
        return;
    };
    while !d.closed {
        d = match d.at {
            None => match wake.wait(d) {
                Ok(d) => d,
                Err(_) => return,
            },
            Some(at) if Instant::now() >= at => {
                d.at = Some(Instant::now() + CANCEL_RETRY);
                log::info!("服务没有及时回复，断开重连");
                // SAFETY: 句柄是发请求的线程，由本线程持有。
                let _ = unsafe { CancelSynchronousIo(thread.raw()) };
                d
            }
            Some(at) => match wake.wait_timeout(d, at.saturating_duration_since(Instant::now())) {
                Ok((d, _)) => d,
                Err(_) => return,
            },
        };
    }
}

/// 当前线程的真实句柄（伪句柄只在本线程有意义，交给别的线程要复制一份）。
fn current_thread() -> Option<OwnedHandle> {
    let mut handle = HANDLE::default();
    // SAFETY: 复制当前线程的伪句柄到本进程，输出到本地变量。
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &mut handle,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .ok()?;
    Some(OwnedHandle::from_raw(handle))
}
