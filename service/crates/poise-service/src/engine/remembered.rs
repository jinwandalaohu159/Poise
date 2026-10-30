//! 运行过的程序（程序偏好页"未运行"一栏的来源）：每 [`REMEMBER_EVERY_MS`] 记一次正在运行的程序，
//! 有变化时保存到数据目录的 seen_apps.json；启动时去掉太久没运行、文件已不存在的。记录规则见
//! `poise_core::seen_apps`。

use std::path::Path;

use poise_core::model::Millis;
use poise_core::seen_apps::SeenApps;
use poise_platform::persist::storage::JsonFile;

use super::unix_now_ms;

const FILE: &str = "seen_apps.json";
/// 多久记录一次运行的程序：只用于"未运行"一栏，不需要每轮都整理一遍分组。
const REMEMBER_EVERY_MS: Millis = 30_000;

pub(super) struct RememberedApps {
    apps: SeenApps,
    file: JsonFile<SeenApps>,
    checked_at: Option<Millis>,
}

impl RememberedApps {
    /// 读回保存的记录，去掉太久没运行、文件已不存在的程序。
    pub fn open(data_dir: &Path) -> Self {
        let file = JsonFile::new(data_dir.join(FILE));
        let mut r = Self {
            apps: file.load().unwrap_or_default(),
            file,
            checked_at: None,
        };
        if r.apps
            .prune(unix_now_ms() / 1000, |p| Path::new(p).exists())
        {
            r.save();
        }
        r
    }

    /// 该记录了吗（距上次至少 [`REMEMBER_EVERY_MS`]）；是的话记下这次的时间。
    pub fn due(&mut self, now: Millis) -> bool {
        if self
            .checked_at
            .is_some_and(|t| now.saturating_sub(t) < REMEMBER_EVERY_MS)
        {
            return false;
        }
        self.checked_at = Some(now);
        true
    }

    /// 记下正在运行的程序（路径，名称），有变化时保存。
    pub fn observe<'a>(&mut self, running: impl IntoIterator<Item = (&'a str, String)>) {
        let now_s = unix_now_ms() / 1000;
        let mut changed = false;
        for (path, name) in running {
            changed |= self.apps.observe(path, &name, now_s);
        }
        if changed {
            self.save();
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.apps.apps.contains_key(path)
    }

    /// 记下的名称。
    pub fn name(&self, path: &str) -> Option<&str> {
        self.apps.apps.get(path).map(|a| a.name.as_str())
    }

    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.apps.apps.keys()
    }

    fn save(&self) {
        if let Err(e) = self.file.save(&self.apps) {
            log::warn!("保存运行过的程序失败：{e}");
        }
    }
}

/// 没有别的名称时用文件名（去掉扩展名）。
pub(super) fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}
