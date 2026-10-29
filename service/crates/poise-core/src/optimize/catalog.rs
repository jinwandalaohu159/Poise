//! 固定的优化项目录。动态项（右键插件、导航栏入口、启动项）由平台层枚举。

use super::model::{Group, Matcher, OptimizationItem, SystemDefault, TakesEffect, Text};
use crate::power_settings as power;
use crate::setting::{Hive, Target, UserFile, Value};

const EXPLORER_ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

fn key(k: &str) -> Text {
    Text::Key(k.to_string())
}

pub fn static_items(wsl_installed: bool) -> Vec<OptimizationItem> {
    let mut items = power_items();
    items.extend(other_items(wsl_installed));
    items
}

/// 电源计划里的项：处理器该降就降、该升就升。它们会被厂商工具、系统更新悄悄改回，
/// 服务定期检查（见 [`super::reverted_power_items`]）。
pub fn power_items() -> Vec<OptimizationItem> {
    vec![
        OptimizationItem {
            // "高性能"等电源计划把插电时的最小处理器状态设为 100%：处理器从不降频，空闲时也一直发热。
            id: "cpu_idle_downclock".into(),
            group: Group::Power,
            title: key("opt.cpu_idle.title"),
            description: key("opt.cpu_idle.desc"),
            targets: power::MIN_STATE
                .iter()
                .map(|s| power::target(s, true))
                .collect(),
            optimized: Value::Dword(power::IDLE_MIN_STATE),
            matcher: Matcher::AtMost(power::IDLE_MIN_STATE_LIMIT),
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::Immediately,
            // 默认值取决于电源计划。
            system_default: SystemDefault::Unknown,
        },
        OptimizationItem {
            // 睿频被关掉时处理器最高只跑到基础频率（i7-12700H 是 2.3 GHz，睿频可到 4.7 GHz），
            // 有负载时也升不上去。只要不是"禁用"就算已优化，不改用户选的其他睿频方式。
            id: "cpu_boost_allowed".into(),
            group: Group::Power,
            title: key("opt.cpu_boost.title"),
            description: key("opt.cpu_boost.desc"),
            targets: vec![power::target(power::BOOST_MODE, true)],
            optimized: Value::Dword(power::BOOST_AGGRESSIVE),
            matcher: Matcher::AtLeast(power::BOOST_DISABLED + 1),
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::Immediately,
            system_default: SystemDefault::Unknown,
        },
    ]
}

fn other_items(wsl_installed: bool) -> Vec<OptimizationItem> {
    let mut items = vec![
        OptimizationItem {
            id: "fast_startup_off".into(),
            group: Group::StartupBackground,
            title: key("opt.fast_startup.title"),
            description: key("opt.fast_startup.desc"),
            targets: vec![Target::Registry {
                hive: Hive::LocalMachine,
                key: r"SYSTEM\CurrentControlSet\Control\Session Manager\Power".into(),
                name: "HiberbootEnabled".into(),
            }],
            optimized: Value::Dword(0),
            matcher: Matcher::Exact,
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::NextRestart,
            system_default: SystemDefault::Value(Value::Dword(1)),
        },
        OptimizationItem {
            id: "explorer_open_this_pc".into(),
            group: Group::Explorer,
            title: key("opt.open_this_pc.title"),
            description: key("opt.open_this_pc.desc"),
            targets: vec![Target::Registry {
                hive: Hive::CurrentUser,
                key: EXPLORER_ADVANCED.into(),
                name: "LaunchTo".into(),
            }],
            optimized: Value::Dword(1),
            matcher: Matcher::Exact,
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::NextFolderWindow,
            system_default: SystemDefault::Absent,
        },
        OptimizationItem {
            id: "explorer_separate_process".into(),
            group: Group::Explorer,
            title: key("opt.separate_process.title"),
            description: key("opt.separate_process.desc"),
            targets: vec![Target::Registry {
                hive: Hive::CurrentUser,
                key: EXPLORER_ADVANCED.into(),
                name: "SeparateProcess".into(),
            }],
            optimized: Value::Dword(1),
            matcher: Matcher::Exact,
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::NextSignIn,
            system_default: SystemDefault::Value(Value::Dword(0)),
        },
    ];
    if wsl_installed {
        items.push(OptimizationItem {
            id: "wsl_memory_reclaim".into(),
            group: Group::Memory,
            title: key("opt.wsl_reclaim.title"),
            description: key("opt.wsl_reclaim.desc"),
            targets: vec![Target::Ini {
                file: UserFile::WslConfig,
                section: "experimental".into(),
                key: "autoMemoryReclaim".into(),
            }],
            // 温和回收：用户平时会在后台运行 WSL/Docker，不用激进的缓存清除。
            optimized: Value::String("gradual".into()),
            matcher: Matcher::Exact,
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::NextRestart,
            // 没有这一项时 WSL 用自己的默认值。
            system_default: SystemDefault::Absent,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_wsl_is_conditional() {
        let with = static_items(true);
        let without = static_items(false);
        assert_eq!(with.len(), without.len() + 1);
        let mut ids: Vec<_> = with.iter().map(|i| &i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), with.len());
    }

    #[test]
    fn forced_full_clock_is_not_optimized_but_balanced_defaults_are() {
        let item = static_items(false)
            .into_iter()
            .find(|i| i.id == "cpu_idle_downclock")
            .unwrap();
        let optimized = |v| {
            item.matcher
                .is_optimized(Some(&Value::Dword(v)), &item.optimized)
        };
        assert!(!optimized(100));
        assert!(optimized(5));
        assert!(optimized(0));
    }

    #[test]
    fn idle_downclock_covers_every_core_class() {
        let item = power_items()
            .into_iter()
            .find(|i| i.id == "cpu_idle_downclock")
            .unwrap();
        // 只改等级 0 时，大小核处理器的大核（等级 1）仍被"高性能"计划锁在最高频率。
        let expected: Vec<Target> = power::MIN_STATE
            .iter()
            .map(|s| power::target(s, true))
            .collect();
        assert_eq!(item.targets, expected);
        assert!(power_items().iter().all(|i| i.group == Group::Power));
    }

    #[test]
    fn disabled_turbo_is_not_optimized_but_any_other_boost_mode_is() {
        let item = power_items()
            .into_iter()
            .find(|i| i.id == "cpu_boost_allowed")
            .unwrap();
        assert!(!item.is_optimized(Some(&Value::Dword(0))));
        assert!(item.is_optimized(Some(&Value::Dword(2))));
        assert!(item.is_optimized(Some(&Value::Dword(4))));
    }

    #[test]
    fn one_click_never_includes_per_item_groups() {
        assert!(static_items(true).iter().all(|i| !matches!(
            i.group,
            Group::ContextMenu | Group::NavigationPane | Group::StartupApps
        ) || !i.in_one_click));
    }
}
