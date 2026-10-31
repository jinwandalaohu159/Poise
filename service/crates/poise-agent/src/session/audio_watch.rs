//! 在单独的线程里查"正在发声"的进程（见 [`super::audio`]）。查询是跨进程的 COM 调用：音频服务或某个
//! 声卡驱动卡住时会一直不返回。放在上报会话的线程里的话，上报跟着停下，20 秒后服务就当托盘代理退出了、
//! 撤销一切。所以每次心跳最多等它 [`ANSWER_WITHIN`]，没等到就沿用上一次的结果；上一次的还没回来时
//! 不再排新的查询。

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use poise_core::model::Pid;
use poise_platform::act::self_tune;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use super::audio::AudioSessions;

/// 每次最多等音频查询这么久（平时约 1 毫秒）。
const ANSWER_WITHIN: Duration = Duration::from_secs(1);

pub struct AudioWatch {
    requests: Sender<()>,
    replies: Receiver<Vec<Pid>>,
    /// 发出的查询还没回来。
    pending: bool,
    /// 最近一次查到的。
    last: Vec<Pid>,
    /// 已经记过"没有及时回应"（恢复之前不再记）。
    stalled: bool,
}

impl AudioWatch {
    pub fn start() -> Self {
        let (requests, asked) = mpsc::channel::<()>();
        let (answer, replies) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                // SAFETY: 本线程加入多线程套间，线程结束前一直有效。
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                // 和上报会话的线程一样及时运行：不在小核上排队。
                self_tune::responsive_thread();
                // COM 对象只能在这个线程里用，所以在这里创建。
                let mut sessions = AudioSessions::default();
                for () in asked {
                    if answer.send(sessions.active_pids()).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("创建音频查询线程失败：{e}");
        }
        Self {
            requests,
            replies,
            pending: false,
            last: Vec::new(),
            stalled: false,
        }
    }

    /// 正在发声的进程：查一次，最多等 [`ANSWER_WITHIN`]；没等到时沿用上一次的。
    pub fn active_pids(&mut self) -> Vec<Pid> {
        if self.pending
            && let Ok(pids) = self.replies.try_recv()
        {
            self.answered(pids);
        }
        if !self.pending && self.requests.send(()).is_ok() {
            self.pending = true;
            match self.replies.recv_timeout(ANSWER_WITHIN) {
                Ok(pids) => self.answered(pids),
                Err(RecvTimeoutError::Timeout) => {
                    if !self.stalled {
                        self.stalled = true;
                        log::warn!("音频服务没有及时回应，先沿用上一次查到的发声程序");
                    }
                }
                Err(RecvTimeoutError::Disconnected) => self.pending = false,
            }
        }
        self.last.clone()
    }

    fn answered(&mut self, pids: Vec<Pid>) {
        self.last = pids;
        self.pending = false;
        if self.stalled {
            self.stalled = false;
            log::info!("音频服务恢复回应");
        }
    }
}
