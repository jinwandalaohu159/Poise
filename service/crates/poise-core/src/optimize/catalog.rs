//! 固定的优化项目录。动态项（右键插件、导航栏入口、启动项）由平台层枚举。

use super::model::{
    Group, Hive, Matcher, OptimizationItem, SystemDefault, TakesEffect, Target, Text, UserFile,
    Value,
};

const EXPLORER_ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
/// 电源计划：处理器电源管理子组、最小处理器状态。
const SUB_PROCESSOR: &str = "54533251-82be-4824-96c1-47b60b740d00";
const PROC_THROTTLE_MIN: &str = "893dee8e-2bef-41e0-89c6-b55d0929964c";
/// Windows"平衡"方案的默认值。
const IDLE_MIN_STATE_PCT: u32 = 5;
/// 不超过这个值就不算"强制高频"。
const IDLE_MIN_STATE_LIMIT: u32 = 50;

fn key(k: &str) -> Text {
    Text::Key(k.to_string())
}

pub fn static_items(wsl_installed: bool) -> Vec<OptimizationItem> {
    let mut items = vec![
        OptimizationItem {
            // "高性能"等电源计划把插电时的最小处理器状态设为 100%：处理器从不降频，空闲时也一直发热。
            id: "cpu_idle_downclock".into(),
            group: Group::Power,
            title: key("opt.cpu_idle.title"),
            description: key("opt.cpu_idle.desc"),
            target: Target::PowerSetting {
                subgroup: SUB_PROCESSOR.into(),
                setting: PROC_THROTTLE_MIN.into(),
                ac: true,
            },
            optimized: Value::Dword(IDLE_MIN_STATE_PCT),
            matcher: Matcher::AtMost(IDLE_MIN_STATE_LIMIT),
            in_one_click: true,
            slow_hint: false,
            takes_effect: TakesEffect::Immediately,
            // 默认值取决于电源计划。
            system_default: SystemDefault::Unknown,
        },
        OptimizationItem {
            id: "fast_startup_off".into(),
            group: Group::StartupBackground,
            title: key("opt.fast_startup.title"),
            description: key("opt.fast_startup.desc"),
            target: Target::Registry {
                hive: Hive::LocalMachine,
                key: r"SYSTEM\CurrentControlSet\Control\Session Manager\Power".into(),
                name: "HiberbootEnabled".into(),
            },
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
            target: Target::Registry {
                hive: Hive::CurrentUser,
                key: EXPLORER_ADVANCED.into(),
                name: "LaunchTo".into(),
            },
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
            target: Target::Registry {
                hive: Hive::CurrentUser,
                key: EXPLORER_ADVANCED.into(),
                name: "SeparateProcess".into(),
            },
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
            target: Target::Ini {
                file: UserFile::WslConfig,
                section: "experimental".into(),
                key: "autoMemoryReclaim".into(),
            },
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
    fn one_click_never_includes_per_item_groups() {
        assert!(static_items(true).iter().all(|i| !matches!(
            i.group,
            Group::ContextMenu | Group::NavigationPane | Group::StartupApps
        ) || !i.in_one_click));
    }
}
