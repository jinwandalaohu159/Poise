//! 运行过的程序：程序偏好页"未运行"一栏的来源，方便在程序没运行时也能设置类别。
//! 保存在本机的数据目录里，升级、重装后保留。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 超过这么久没再运行的程序不再列出（设置过类别的程序另由偏好列出，不受影响）。
const KEEP_FOR_S: u64 = 90 * 24 * 3600;
/// 最多记住这么多个程序，超过时去掉最久没运行的。
const MAX_APPS: usize = 300;
/// 最近运行时间只按天更新，避免频繁写盘。
const TOUCH_EVERY_S: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenApp {
    /// 给用户看的名称（程序的文件描述）。
    pub name: String,
    /// 最近一次看到它在运行（Unix 秒，按天更新）。
    pub last_seen_s: u64,
}

/// 按程序路径（小写）记录。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeenApps {
    pub apps: BTreeMap<String, SeenApp>,
}

impl SeenApps {
    /// 记录一个正在运行的程序。返回是否有需要保存的变化（新程序、改名、隔天再次运行）。
    pub fn observe(&mut self, path: &str, name: &str, now_s: u64) -> bool {
        match self.apps.get_mut(path) {
            Some(app) => {
                let renamed = app.name != name;
                let stale = now_s.saturating_sub(app.last_seen_s) >= TOUCH_EVERY_S;
                if renamed {
                    app.name = name.to_string();
                }
                if stale {
                    app.last_seen_s = now_s;
                }
                renamed || stale
            }
            None => {
                self.apps.insert(
                    path.to_string(),
                    SeenApp {
                        name: name.to_string(),
                        last_seen_s: now_s,
                    },
                );
                self.trim();
                true
            }
        }
    }

    /// 去掉太久没运行的、程序文件已经不存在的。返回是否有变化。
    pub fn prune(&mut self, now_s: u64, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.apps.len();
        self.apps
            .retain(|path, app| now_s.saturating_sub(app.last_seen_s) < KEEP_FOR_S && exists(path));
        self.apps.len() != before
    }

    fn trim(&mut self) {
        while self.apps.len() > MAX_APPS {
            let oldest = self
                .apps
                .iter()
                .min_by_key(|(_, a)| a.last_seen_s)
                .map(|(p, _)| p.clone());
            match oldest {
                Some(p) => self.apps.remove(&p),
                None => break,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    #[test]
    fn saves_only_when_something_worth_keeping_changed() {
        let mut s = SeenApps::default();
        assert!(s.observe(r"c:\apps\a.exe", "A", 100));
        // 同一天再次看到：不需要保存。
        assert!(!s.observe(r"c:\apps\a.exe", "A", 100 + 3600));
        assert!(s.observe(r"c:\apps\a.exe", "A 新名称", 200));
        assert!(s.observe(r"c:\apps\a.exe", "A 新名称", 100 + DAY));
    }

    #[test]
    fn prunes_old_and_missing_programs() {
        let mut s = SeenApps::default();
        s.observe("old.exe", "Old", 0);
        s.observe("gone.exe", "Gone", 100 * DAY);
        s.observe("kept.exe", "Kept", 100 * DAY);
        assert!(s.prune(100 * DAY, |p| p != "gone.exe"));
        assert_eq!(s.apps.keys().collect::<Vec<_>>(), ["kept.exe"]);
    }

    #[test]
    fn keeps_at_most_the_most_recent_programs() {
        let mut s = SeenApps::default();
        for i in 0..=MAX_APPS as u64 {
            s.observe(&format!("{i}.exe"), "App", i);
        }
        assert_eq!(s.apps.len(), MAX_APPS);
        assert!(!s.apps.contains_key("0.exe"));
    }
}
