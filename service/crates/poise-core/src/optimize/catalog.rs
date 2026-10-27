//! 固定的优化项目录。动态项（右键插件、导航栏入口、启动项）由平台层枚举。

use super::model::{
    Group, Hive, Matcher, OptimizationItem, TakesEffect, Target, Text, UserFile, Value,
};

const EXPLORER_ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

fn key(k: &str) -> Text {
    Text::Key(k.to_string())
}

pub fn static_items(wsl_installed: bool) -> Vec<OptimizationItem> {
    let mut items = vec![
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
    fn one_click_never_includes_per_item_groups() {
        assert!(static_items(true).iter().all(|i| !matches!(
            i.group,
            Group::ContextMenu | Group::NavigationPane | Group::StartupApps
        ) || !i.in_one_click));
    }
}
