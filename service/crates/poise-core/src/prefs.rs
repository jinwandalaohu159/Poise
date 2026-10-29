//! 用户设置：模式与程序偏好。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::ProcessFacts;

/// 模式。用户可以选择全部五种；实际生效的模式不会是 [`Mode::Auto`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 由 Poise 按负载、空闲、温度、供电在安静 / 均衡 / 全力之间切换。
    #[default]
    Auto,
    Quiet,
    Balanced,
    Full,
    /// 机型专属的极限模式（戴尔 G 模式、华硕天选模式等）。只有机型档案支持时才可用。
    Vendor,
}

impl Mode {
    /// 性能高低（安静 0，均衡 1，全力 2，极限 3）；"自动"按均衡计。
    pub fn rank(self) -> u8 {
        match self {
            Self::Quiet => 0,
            Self::Auto | Self::Balanced => 1,
            Self::Full => 2,
            Self::Vendor => 3,
        }
    }
}

/// 玩游戏时切换到哪个模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    /// 不切换，保持用户选择的模式。
    Off,
    #[default]
    Full,
    /// 机型极限模式；机型不支持时按全力处理。
    Vendor,
}

impl GameMode {
    /// 游戏时的目标模式。`vendor_available` 为假时极限模式退回全力。
    pub fn target(self, vendor_available: bool) -> Option<Mode> {
        match self {
            Self::Off => None,
            Self::Vendor if vendor_available => Some(Mode::Vendor),
            Self::Full | Self::Vendor => Some(Mode::Full),
        }
    }
}

/// 程序的调度类别（界面上的"自动 / 始终优先 / 低优先级 / 游戏"）。
/// 变体名会写进设置文件和协议，保持不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppPreference {
    /// 自动：在后台时让出性能，到前台立即提速。
    #[default]
    Auto,
    /// 始终优先：在后台也保持高性能。
    AlwaysFast,
    /// 低优先级：始终在小核上运行，到前台也不提速。
    Background,
    /// 游戏：始终最高性能，并切换到"游戏时使用"的模式。
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
    /// 玩游戏时切换到的模式。
    pub game_mode: GameMode,
    /// 自动清理（只做无感项目）。
    pub auto_clean: bool,
    /// 风扇加速，0～100（0 = 不加速，由 BIOS 按温度调节）。
    pub fan_boost: u8,
    /// 智能调度：按程序类别和前台/后台分配核心与优先级。关闭时不调整任何进程。
    pub smart_schedule: bool,
    pub preferences: Preferences,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            game_mode: GameMode::Full,
            auto_clean: true,
            fan_boost: 0,
            smart_schedule: true,
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
