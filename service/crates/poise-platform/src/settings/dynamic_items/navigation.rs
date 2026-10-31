//! 资源管理器导航栏里的第三方入口（网盘等）：取消固定即不再显示。

use poise_core::optimize::{Group, Matcher, OptimizationItem, SystemDefault, TakesEffect, Text};
use poise_core::setting::{Hive, Target, Value};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use super::{class_roots, is_guid};
use crate::settings::user::UserContext;
use crate::win::registry;

const NAMESPACE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace";

pub fn navigation_items(user: Option<&UserContext>) -> Vec<OptimizationItem> {
    let roots = class_roots(user);
    let mut namespace_roots = vec![(HKEY_LOCAL_MACHINE, NAMESPACE_KEY.to_string())];
    if let Some(u) = user {
        let (root, prefix) = u.registry_root();
        namespace_roots.push((root, format!("{prefix}{NAMESPACE_KEY}")));
    }
    let mut clsids: Vec<String> = namespace_roots
        .iter()
        .filter_map(|(root, path)| registry::open(*root, path, false).ok())
        .flat_map(|k| k.subkeys())
        .filter(|s| is_guid(s))
        .map(|s| s.to_lowercase())
        .collect();
    clsids.sort();
    clsids.dedup();
    clsids
        .into_iter()
        .filter_map(|clsid| {
            let keys: Vec<_> = roots
                .iter()
                .filter_map(|(root, prefix)| {
                    registry::open(*root, &format!(r"{prefix}CLSID\{clsid}"), false).ok()
                })
                .collect();
            // 用户自己的设置优先于本机设置。
            let pinned = keys
                .iter()
                .find_map(|k| match k.read("System.IsPinnedToNameSpaceTree") {
                    Ok(Some(Value::Dword(v))) => Some(v),
                    _ => None,
                })
                == Some(1);
            // 只列出有可读名称的第三方入口（系统自带的"主页""图库"名称是资源引用）。
            let name = keys
                .iter()
                .find_map(|k| k.default_string())
                .filter(|n| !n.starts_with('@'))?;
            if !pinned {
                return None;
            }
            Some(OptimizationItem {
                id: format!("nav:{clsid}"),
                group: Group::NavigationPane,
                title: Text::Plain(name),
                description: Text::Key("opt.nav.desc".into()),
                targets: vec![Target::registry(
                    Hive::CurrentUser,
                    format!(r"Software\Classes\CLSID\{clsid}"),
                    "System.IsPinnedToNameSpaceTree",
                )],
                optimized: Value::Dword(0),
                matcher: Matcher::Exact,
                in_one_click: false,
                slow_hint: false,
                takes_effect: TakesEffect::NextFolderWindow,
                system_default: SystemDefault::Unknown,
            })
        })
        .collect()
}
