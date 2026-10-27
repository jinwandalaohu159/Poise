//! 用户设置：模式与程序偏好。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::ProcessFacts;

/// 用户选择的模式（界面上的三档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Quiet,
    #[default]
    Balanced,
    Full,
}

/// 程序偏好（界面上的"自动 / 一直保持快 / 别打扰我 / 这是游戏"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppPreference {
    #[default]
    Auto,
    AlwaysFast,
    Background,
    Game,
}

/// 按程序路径（小写）记录的偏好。没有路径时退回到文件名匹配。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub apps: BTreeMap<String, AppPreference>,
}

impl Preferences {
    pub fn lookup(&self, facts: &ProcessFacts) -> AppPreference {
        if let Some(path) = &facts.image_path
            && let Some(p) = self.apps.get(path)
        {
            return *p;
        }
        self.apps.get(&facts.name).copied().unwrap_or_default()
    }

    /// 设置偏好；设为 Auto 等同于删除。
    pub fn set(&mut self, key: &str, pref: AppPreference) {
        let key = key.to_lowercase();
        if pref == AppPreference::Auto {
            self.apps.remove(&key);
        } else {
            self.apps.insert(key, pref);
        }
    }
}

/// 服务的持久设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// 玩游戏时自动全力。
    pub auto_full_for_games: bool,
    /// 自动清理（只做无感项目）。
    pub auto_clean: bool,
    pub preferences: Preferences,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Balanced,
            auto_full_for_games: true,
            auto_clean: true,
            preferences: Preferences::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(name: &str, path: Option<&str>) -> ProcessFacts {
        ProcessFacts {
            name: name.into(),
            image_path: path.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn path_match_wins_over_name() {
        let mut p = Preferences::default();
        p.set(r"C:\Games\Foo.exe", AppPreference::Game);
        p.set("foo.exe", AppPreference::Background);
        assert_eq!(
            p.lookup(&facts("foo.exe", Some(r"c:\games\foo.exe"))),
            AppPreference::Game
        );
        assert_eq!(
            p.lookup(&facts("foo.exe", Some(r"d:\other\foo.exe"))),
            AppPreference::Background
        );
    }

    #[test]
    fn setting_auto_removes_entry() {
        let mut p = Preferences::default();
        p.set("a.exe", AppPreference::AlwaysFast);
        p.set("a.exe", AppPreference::Auto);
        assert!(p.apps.is_empty());
    }
}
