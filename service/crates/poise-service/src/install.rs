//! 安装 / 卸载（需要管理员权限，由安装程序调用）。
//!
//! - 安装：注册并启动 Windows 服务（意外结束后自动重启）；已经安装过时（升级、换了安装位置）更新服务的
//!   程序路径并启动。首次安装时为当前用户开启开机启动，之后由界面设置页控制。
//! - 卸载：停止并删除服务，还原 Poise 做过的所有修改，删除所有用户的开机启动任务。
//!
//! 复制文件、快捷方式、"设置 → 应用"里的登记由安装程序负责。

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use poise_platform::persist::paths;
use poise_platform::settings::user::UserContext;
use poise_platform::win::task_scheduler;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::engine::changes::Changes;
use crate::host::SERVICE_NAME;

/// 任务计划程序里的文件夹：每个开启了开机启动的用户一个任务，卸载时整个删除。
const TASK_FOLDER: &str = "Poise";
/// 任务定义（与界面设置页共用），与本程序在同一目录。
const TASK_TEMPLATE: &str = "AutostartTask.xml";

pub fn install() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = service_info(&exe);
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG | ServiceAccess::START;
    let (service, fresh) = match manager.open_service(SERVICE_NAME, access) {
        Ok(existing) => {
            existing.change_config(&info)?;
            (existing, false)
        }
        Err(_) => (manager.create_service(&info, access)?, true),
    };
    service.set_description("让正在操作的程序始终反应迅速：智能调度、清理与系统优化。")?;
    if let Err(e) = restart_on_failure(&service) {
        eprintln!("设置失败后自动重启没有成功：{e}");
    }
    start(&service)?;
    if fresh && let Err(e) = enable_autostart(&exe) {
        // 开机启动没开成不影响使用，用户可以在设置页里再开。
        eprintln!("开机启动没有开启：{e}");
    }
    println!(
        "Poise 服务已{}并启动。",
        if fresh { "安装" } else { "更新" }
    );
    Ok(())
}

/// 停止并删除服务，然后还原所有修改。服务已经不存在时只做还原（用于重试）。
/// 有修改没能还原时返回错误：调用方据此保留修改记录，之后可以再次运行重试。
pub fn uninstall() -> anyhow::Result<()> {
    remove_service()?;
    // 按变更日志还原之前先确认日志是可信的（见 `paths::ensure_dir`）。
    if let Err(e) = paths::ensure_dir(&paths::service_data_dir(), true) {
        eprintln!("收紧数据目录权限失败：{e}");
    }
    let mut changes = Changes::new(
        &paths::service_data_dir(),
        UserContext::current_process_user(),
        false,
    );
    let failed = changes.revert_everything();
    anyhow::ensure!(
        failed.is_empty(),
        "以下修改未能还原（记录已保留，可稍后再次运行 uninstall 重试）：{failed:?}"
    );
    if let Err(e) = task_scheduler::delete_folder(TASK_FOLDER) {
        eprintln!("删除开机启动任务失败：{e}");
    }
    println!("Poise 服务已卸载，所有修改都已还原。");
    Ok(())
}

fn service_info(exe: &Path) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Poise"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_path_buf(),
        launch_arguments: vec![OsString::from("run")],
        dependencies: vec![],
        // None = LocalSystem。
        account_name: None,
        account_password: None,
    }
}

/// 服务意外结束（崩溃、出错退出）后自动重启：5 秒后、30 秒后，之后每次 2 分钟；一天没再出错就从头算。
/// 重启后按调整记录把上次来不及撤销的进程恢复原样。
fn restart_on_failure(service: &Service) -> windows_service::Result<()> {
    let restart = |secs| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(secs),
    };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(30), restart(120)]),
    })?;
    // 以非零退出码正常结束（运行出错）也按失败处理。
    service.set_failure_actions_on_non_crash_failures(true)
}

fn start(service: &Service) -> anyhow::Result<()> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        service.start::<&str>(&[])?;
    }
    Ok(())
}

/// 当前用户登录时启动托盘代理。安装程序以登录用户的身份（经管理员确认）运行，通常就是要开启的用户。
fn enable_autostart(exe: &Path) -> anyhow::Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("找不到安装目录"))?;
    let sid =
        UserContext::current_process_sid().ok_or_else(|| anyhow::anyhow!("读不到当前用户"))?;
    let template = std::fs::read_to_string(dir.join(TASK_TEMPLATE))?;
    let agent = dir.join("poise-agent.exe");
    let xml = template
        .replace("{UserSid}", &xml_escape(&sid))
        .replace("{AgentPath}", &xml_escape(&agent.to_string_lossy()));
    task_scheduler::register(TASK_FOLDER, &format!("Agent {sid}"), &xml)?;
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn remove_service() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) else {
        println!("Poise 服务不存在，只还原修改。");
        return Ok(());
    };
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
        let start = Instant::now();
        while service.query_status()?.current_state != ServiceState::Stopped
            && start.elapsed() < Duration::from_secs(20)
        {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    service.delete()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_escaped_for_the_task_definition() {
        assert_eq!(
            xml_escape(r#"D:\R&D\<x>\"a'.exe"#),
            r"D:\R&amp;D\&lt;x&gt;\&quot;a&apos;.exe"
        );
    }
}
