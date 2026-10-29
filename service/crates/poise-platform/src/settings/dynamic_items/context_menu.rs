//! 右键菜单插件：第三方的、每次右键都可能被加载的插件，以及卸载软件后残留的注册。
//! 关闭方式与系统相同：写进"已屏蔽的外壳扩展"列表。

use std::collections::BTreeMap;

use poise_core::optimize::{Group, Matcher, OptimizationItem, SystemDefault, TakesEffect, Text};
use poise_core::setting::{Hive, Target, Value};
use windows::Win32::System::Registry::HKEY;

use super::{class_roots, expand_env, is_guid, is_system_file};
use crate::settings::user::UserContext;
use crate::win::{registry, version_info};

const BLOCKED_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked";

/// 右键菜单插件注册的位置。前两个在"右键任何文件 / 文件夹"时都会加载。
const MENU_LOCATIONS: &[&str] = &[
    r"*\shellex\ContextMenuHandlers",
    r"Directory\shellex\ContextMenuHandlers",
    r"Directory\Background\shellex\ContextMenuHandlers",
    r"Folder\shellex\ContextMenuHandlers",
    r"AllFilesystemObjects\shellex\ContextMenuHandlers",
    r"Drive\shellex\ContextMenuHandlers",
];

/// 一个右键插件（按 CLSID 合并多个注册位置）。
struct Handler {
    names: Vec<String>,
    locations: Vec<&'static str>,
}

pub fn context_menu_items(user: Option<&UserContext>) -> Vec<OptimizationItem> {
    let roots = class_roots(user);
    let mut handlers: BTreeMap<String, Handler> = BTreeMap::new();
    for (root, prefix) in &roots {
        for loc in MENU_LOCATIONS {
            let Ok(key) = registry::open(*root, &format!("{prefix}{loc}"), false) else {
                continue;
            };
            for name in key.subkeys() {
                let clsid = if is_guid(&name) {
                    Some(name.clone())
                } else {
                    registry::open(*root, &format!(r"{prefix}{loc}\{name}"), false)
                        .ok()
                        .and_then(|k| k.default_string())
                        .filter(|v| is_guid(v))
                };
                let Some(clsid) = clsid else { continue };
                let h = handlers.entry(clsid.to_lowercase()).or_insert(Handler {
                    names: Vec::new(),
                    locations: Vec::new(),
                });
                if !h.names.contains(&name) {
                    h.names.push(name.clone());
                }
                if !h.locations.contains(loc) {
                    h.locations.push(loc);
                }
            }
        }
    }
    handlers
        .into_iter()
        .filter_map(|(clsid, h)| {
            // 在"任何文件"和"文件夹"上都注册的插件，每次右键都会被加载。
            let everywhere = h.locations.contains(&MENU_LOCATIONS[0])
                && (h.locations.contains(&MENU_LOCATIONS[1])
                    || h.locations.contains(&MENU_LOCATIONS[3]));
            let (title, description) = match server_path(&roots, &clsid) {
                Some(dll) => {
                    if is_system_file(&dll) {
                        return None;
                    }
                    let v = version_info::read(&dll);
                    if v.company_name
                        .as_deref()
                        .is_some_and(|c| c.contains("Microsoft"))
                    {
                        return None;
                    }
                    let title = v
                        .file_description
                        .clone()
                        .or(v.product_name.clone())
                        .unwrap_or_else(|| h.names[0].clone());
                    (
                        title,
                        Text::Plain(v.product_name.or(v.company_name).unwrap_or_default()),
                    )
                }
                // 注册还在，但对应的组件已经不存在：通常是卸载软件后的残留。
                None if !clsid_exists(&roots, &clsid) => {
                    (h.names[0].clone(), Text::Key("opt.ctx.orphan".into()))
                }
                None => return None,
            };
            Some(OptimizationItem {
                id: format!("ctx:{clsid}"),
                group: Group::ContextMenu,
                title: Text::Plain(title),
                description,
                targets: vec![Target::Registry {
                    hive: Hive::LocalMachine,
                    key: BLOCKED_KEY.into(),
                    name: clsid.to_uppercase(),
                }],
                optimized: Value::String(String::new()),
                matcher: Matcher::Present,
                in_one_click: false,
                slow_hint: everywhere,
                takes_effect: TakesEffect::NextSignIn,
                // 不在屏蔽列表里即为默认（插件照常加载）。
                system_default: SystemDefault::Absent,
            })
        })
        .collect()
}

fn clsid_exists(roots: &[(HKEY, String)], clsid: &str) -> bool {
    roots.iter().any(|(root, prefix)| {
        registry::open(*root, &format!(r"{prefix}CLSID\{clsid}"), false).is_ok()
    })
}

fn server_path(roots: &[(HKEY, String)], clsid: &str) -> Option<String> {
    roots.iter().find_map(|(root, prefix)| {
        registry::open(
            *root,
            &format!(r"{prefix}CLSID\{clsid}\InprocServer32"),
            false,
        )
        .ok()
        .and_then(|k| k.default_string())
        .map(|p| expand_env(&p))
    })
}
