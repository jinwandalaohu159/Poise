//! 动态优化项：右键菜单插件、导航栏第三方入口、开机启动项——按这台电脑上实际注册的内容列出。
//! 都是逐个开关的项，不参与一键优化。

mod context_menu;
mod navigation;
mod startup;

pub use context_menu::context_menu_items;
pub use navigation::navigation_items;
pub use startup::startup_items;

use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE};

use crate::persist::paths;
use crate::settings::user::UserContext;

/// 类注册的根：用户自己的优先，其次是本机。
pub(super) fn class_roots(user: Option<&UserContext>) -> Vec<(HKEY, String)> {
    let mut roots = Vec::new();
    if let Some(u) = user {
        let (root, prefix) = u.registry_root();
        roots.push((root, format!(r"{prefix}Software\Classes\")));
    }
    roots.push((HKEY_LOCAL_MACHINE, r"SOFTWARE\Classes\".into()));
    roots
}

pub(super) fn is_guid(s: &str) -> bool {
    s.len() == 38 && s.starts_with('{') && s.ends_with('}')
}

pub(super) fn expand_env(path: &str) -> String {
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

pub(super) fn is_system_file(path: &str) -> bool {
    paths::is_in_windows_dir(&path.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

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
