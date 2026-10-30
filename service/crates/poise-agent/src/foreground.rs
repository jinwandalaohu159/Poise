//! 最近一次在前台的"其他程序"：托盘菜单"让 X 全速 1 小时"的对象。
//! 不包括 Poise 自己（托盘代理与界面）和桌面外壳（任务栏、托盘溢出区都属于它）。

use poise_core::model::Pid;
use poise_platform::win::handle::image_path;
use poise_platform::win::version_info;

use crate::session::desktop;
use crate::ui_app;

#[derive(Default)]
pub struct LastForeground {
    pid: Option<Pid>,
}

impl LastForeground {
    /// 前台窗口变化时调用。
    pub fn observe(&mut self) {
        let Some(pid) = desktop::app_pid(desktop::foreground_window()) else {
            return;
        };
        if pid == std::process::id() || Some(pid) == desktop::shell_pid() {
            return;
        }
        if !ui_app::is_ui_process(pid) {
            self.pid = Some(pid);
        }
    }

    /// 对象的进程 ID 和给用户看的名字；进程已经退出时为 None。
    pub fn target(&self) -> Option<(Pid, String)> {
        let pid = self.pid?;
        let path = image_path(pid)?;
        Some((pid, friendly_name(&path)))
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
