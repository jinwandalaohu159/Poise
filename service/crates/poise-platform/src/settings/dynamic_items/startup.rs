//! 开机启动项（Run 键）：用任务管理器同样的"已批准"标记启用 / 禁用，不删除注册。

use poise_core::optimize::{Group, Matcher, OptimizationItem, SystemDefault, TakesEffect, Text};
use poise_core::setting::{Hive, Target, Value};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use super::expand_env;
use crate::settings::user::UserContext;
use crate::win::{registry, version_info};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW_KEY: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// 任务管理器使用的"已禁用"标记（首字节为奇数，后面是时间戳）。
const DISABLED_MARK: [u8; 12] = [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// 任务管理器使用的"已启用"标记。
const ENABLED_MARK: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

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
                targets: vec![Target::Registry {
                    hive,
                    key: format!(r"{APPROVED_KEY}\{approved}"),
                    name,
                }],
                optimized: Value::Binary(DISABLED_MARK.to_vec()),
                matcher: Matcher::FirstByteOdd,
                in_one_click: false,
                slow_hint: false,
                takes_effect: TakesEffect::NextSignIn,
                // 在任务管理器里禁用的启动项，也可以在这里重新启用。
                system_default: SystemDefault::Value(Value::Binary(ENABLED_MARK.to_vec())),
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
}
