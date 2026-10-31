//! 最近一次在前台的"其他程序"：托盘菜单"让 X 全速 1 小时"的对象。
//! 不包括 Poise 自己（托盘代理与界面）和桌面外壳（任务栏、托盘溢出区都属于它）。
//!
//! 前台每次切换只记下进程号（在托盘线程上，要快）；是不是 Poise 的界面、叫什么名字，弹菜单时才查，
//! 名字查到后记住（要读程序文件的版本信息，再弹菜单时不再读）。

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use poise_core::model::Pid;
use poise_platform::win::handle::image_path;
use poise_platform::win::version_info;

use crate::session::desktop;
use crate::ui_app;

/// 记住最近几个前台程序（最近的在后）：最近的一个可能是 Poise 的界面，往前找。
const REMEMBER: usize = 4;

#[derive(Default)]
pub struct LastForeground {
    recent: VecDeque<Pid>,
    /// 最近几个程序的名字（只留 `recent` 里的）。
    names: RefCell<HashMap<Pid, String>>,
}

impl LastForeground {
    /// 前台窗口变化时调用。
    pub fn observe(&mut self) {
        let Some(pid) = desktop::app_pid(desktop::foreground_window()) else {
            return;
        };
        if pid == std::process::id() || self.recent.back() == Some(&pid) {
            return;
        }
        self.recent.retain(|p| *p != pid);
        if self.recent.len() == REMEMBER
            && let Some(gone) = self.recent.pop_front()
        {
            self.names.borrow_mut().remove(&gone);
        }
        self.recent.push_back(pid);
    }

    /// 对象的进程 ID 和给用户看的名字；都退出了时为 None。
    pub fn target(&self) -> Option<(Pid, String)> {
        let shell = desktop::shell_pid();
        self.recent
            .iter()
            .rev()
            .filter(|pid| Some(**pid) != shell && !ui_app::is_ui_process(**pid))
            .find_map(|&pid| Some((pid, self.name_of(pid)?)))
    }

    fn name_of(&self, pid: Pid) -> Option<String> {
        if let Some(name) = self.names.borrow().get(&pid) {
            return Some(name.clone());
        }
        let name = friendly_name(&image_path(pid)?);
        self.names.borrow_mut().insert(pid, name.clone());
        Some(name)
    }
}

/// 优先用程序自己的描述（"Windows Terminal"），没有时用文件名。
fn friendly_name(path: &str) -> String {
    version_info::read(path)
        .file_description
        .filter(|d| !d.trim().is_empty())
        .map(|d| d.trim().to_string())
        .unwrap_or_else(|| {
            std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}
