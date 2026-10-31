//! 更新流程（后台线程）与进度窗口（界面线程）共享的状态。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Checking,
    Downloading {
        done: u64,
        total: u64,
    },
    /// 安装程序在运行（从什么时候开始）。
    Installing {
        since: Instant,
    },
    /// 安装完成，等待服务与托盘代理启动。`fraction` 是这一段已经走到的进度（0～1）。
    Starting {
        fraction: f32,
    },
    Finished(Outcome),
}

impl Stage {
    /// 第几段进度：0 = 下载，1 = 安装并启动。换段时进度条从头开始。
    pub fn part(&self) -> u8 {
        match self {
            Self::Checking | Self::Downloading { .. } => 0,
            _ => 1,
        }
    }

    /// 还能不能取消：安装开始后不能（安装程序失败时自己回滚，不能从外面打断）。
    pub fn cancellable(&self) -> bool {
        self.part() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Updated,
    UpToDate,
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 连不上 GitHub 或下载中断（细节只写日志）。
    Network(String),
    /// 下载的文件大小或校验值不对。
    Verification,
    /// 用户在系统的权限提示里选了"否"。
    Declined,
    /// 安装程序没能启动（细节只写日志）。
    Launch(String),
    /// 安装程序失败（退出码）。安装程序会回滚，原来的版本仍可用。
    Install(u32),
    /// 安装程序运行了，但没有及时结束或读不到结果：不知道装没装上（细节只写日志）。
    Unfinished(String),
    /// 安装完成但服务没有启动。
    Start,
    /// 用户取消了下载。
    Cancelled,
}

#[derive(Debug)]
struct State {
    /// 要更新到的版本（查询到之后才有）。
    version: Option<String>,
    stage: Stage,
}

#[derive(Debug)]
pub struct Shared {
    state: Mutex<State>,
    cancelled: AtomicBool,
    /// 进度窗口的句柄：系统的权限提示以它为父窗口，显示在它前面。
    owner: AtomicIsize,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                version: None,
                stage: Stage::Checking,
            }),
            cancelled: AtomicBool::new(false),
            owner: AtomicIsize::new(0),
        }
    }
}

impl Shared {
    pub fn snapshot(&self) -> (Option<String>, Stage) {
        let s = self.lock();
        (s.version.clone(), s.stage.clone())
    }

    pub fn set_version(&self, version: &str) {
        self.lock().version = Some(version.to_string());
    }

    pub fn set_stage(&self, stage: Stage) {
        self.lock().stage = stage;
    }

    pub fn finish(&self, outcome: Outcome) {
        self.set_stage(Stage::Finished(outcome));
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn set_owner(&self, hwnd: isize) {
        self.owner.store(hwnd, Ordering::Relaxed);
    }

    pub fn owner(&self) -> isize {
        self.owner.load(Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
