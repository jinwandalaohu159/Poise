//! 动态优化项：右键菜单插件、导航栏第三方入口、开机启动项。
//! 都是逐个开关的项，不参与一键优化。

use std::collections::BTreeMap;

use poise_core::optimize::{
    Group, Hive, Matcher, OptimizationItem, TakesEffect, Target, Text, Value,
};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::settings::user::UserContext;
use crate::win::registry;
use crate::win::version_info;

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

/// 类注册的根：用户自己的优先，其次是本机。
fn class_roots(user: Option<&UserContext>) -> Vec<(HKEY, String)> {
    let mut roots = Vec::new();
    if let Some(u) = user {
        let (root, prefix) = u.registry_root();
        roots.push((root, format!(r"{prefix}Software\Classes\")));
    }
    roots.push((HKEY_LOCAL_MACHINE, r"SOFTWARE\Classes\".into()));
    roots
}

fn is_guid(s: &str) -> bool {
    s.len() == 38 && s.starts_with('{') && s.ends_with('}')
}

fn expand_env(path: &str) -> String {
    let mut out = path.to_string();
    for var in [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "CommonProgramFiles",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        if let Some(v) = std::env::var_os(var) {
            let pat = format!("%{var}%");
            if let Some(i) = out.to_lowercase().find(&pat.to_lowercase()) {
                out.replace_range(i..i + pat.len(), &v.to_string_lossy());
            }
        }
    }
    out.trim_matches('"').to_string()
}

fn is_system_file(path: &str) -> bool {
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    path.to_lowercase().starts_with(&windir.to_lowercase())
}

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
                target: Target::Registry {
                    hive: Hive::LocalMachine,
                    key: BLOCKED_KEY.into(),
                    name: clsid.to_uppercase(),
                },
                optimized: Value::String(String::new()),
                matcher: Matcher::Present,
                in_one_click: false,
                slow_hint: everywhere,
                takes_effect: TakesEffect::NextSignIn,
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
                target: Target::Registry {
                    hive: Hive::CurrentUser,
                    key: format!(r"Software\Classes\CLSID\{clsid}"),
                    name: "System.IsPinnedToNameSpaceTree".into(),
                },
                optimized: Value::Dword(0),
                matcher: Matcher::Exact,
                in_one_click: false,
                slow_hint: false,
                takes_effect: TakesEffect::NextFolderWindow,
            })
        })
        .collect()
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW_KEY: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// 任务管理器使用的"已禁用"标记（首字节为奇数，后面是时间戳）。
const DISABLED_MARK: [u8; 12] = [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub fn startup_items(user: Option<&UserContext>) -> Vec<OptimizationItem> {
    let mut sources: Vec<(Hive, HKEY, String, &str)> = vec![
        (
            Hive::LocalMachine,
            HKEY_LOCAL_MACHINE,
            RUN_KEY.to_string(),
            "Run",
        ),
        (
            Hive::LocalMachine,
            HKEY_LOCAL_MACHINE,
            RUN_WOW_KEY.to_string(),
            "Run32",
        ),
    ];
    if let Some(u) = user {
        let (root, prefix) = u.registry_root();
        sources.push((Hive::CurrentUser, root, format!("{prefix}{RUN_KEY}"), "Run"));
    } else {
        sources.push((
            Hive::CurrentUser,
            HKEY_CURRENT_USER,
            RUN_KEY.to_string(),
            "Run",
        ));
    }
    let mut items = Vec::new();
    for (hive, root, path, approved) in sources {
        let Ok(key) = registry::open(root, &path, false) else {
            continue;
        };
        for (name, value) in key.values() {
            let Value::String(command) = value else {
                continue;
            };
            if name.is_empty() || name.eq_ignore_ascii_case("Poise") {
                continue;
            }
            let exe = exe_from_command(&command);
            let title = exe
                .as_deref()
                .and_then(|p| version_info::read(p).file_description)
                .unwrap_or_else(|| name.clone());
            items.push(OptimizationItem {
                id: format!("startup:{}:{approved}:{name}", hive_tag(hive)),
                group: Group::StartupApps,
                title: Text::Plain(title),
                description: Text::Plain(name.clone()),
                target: Target::Registry {
                    hive,
                    key: format!(r"{APPROVED_KEY}\{approved}"),
                    name,
                },
                optimized: Value::Binary(DISABLED_MARK.to_vec()),
                matcher: Matcher::FirstByteOdd,
                in_one_click: false,
                slow_hint: false,
                takes_effect: TakesEffect::NextSignIn,
            });
        }
    }
    items
}

fn hive_tag(h: Hive) -> &'static str {
    match h {
        Hive::LocalMachine => "lm",
        Hive::CurrentUser => "cu",
    }
}

/// 从启动命令里取出可执行文件路径。
fn exe_from_command(command: &str) -> Option<String> {
    let c = command.trim();
    let path = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let lower = c.to_lowercase();
        let end = lower.find(".exe")? + 4;
        c[..end].to_string()
    };
    Some(expand_env(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exe_from_startup_commands() {
        assert_eq!(
            exe_from_command(r#""C:\Program Files\App\app.exe" --silent"#).as_deref(),
            Some(r"C:\Program Files\App\app.exe")
        );
        assert_eq!(
            exe_from_command(r"C:\Tools\tool.exe /min").as_deref(),
            Some(r"C:\Tools\tool.exe")
        );
        assert_eq!(exe_from_command("rundll32 something"), None);
    }

    #[test]
    fn enumerators_run_read_only_and_ids_are_unique() {
        let user = UserContext::current_process_user();
        let mut all = context_menu_items(user.as_ref());
        all.extend(navigation_items(user.as_ref()));
        all.extend(startup_items(user.as_ref()));
        let mut ids: Vec<_> = all.iter().map(|i| i.id.clone()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert!(all.iter().all(|i| !i.in_one_click));
    }
}
