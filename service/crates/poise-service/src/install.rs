//! 安装 / 卸载 Windows 服务（需要管理员权限）。卸载时还原 Poise 做过的所有修改。

use std::ffi::OsString;
use std::time::{Duration, Instant};

use poise_platform::persist::paths;
use poise_platform::settings::user::UserContext;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::engine::changes::Changes;
use crate::host::SERVICE_NAME;

pub fn install() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Poise"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments: vec![OsString::from("run")],
        dependencies: vec![],
        // None = LocalSystem。
        account_name: None,
        account_password: None,
    };
    let service =
        manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    service.set_description("让正在操作的程序始终反应迅速：智能调度、清理与系统优化。")?;
    service.start::<&str>(&[])?;
    println!("Poise 服务已安装并启动。");
    Ok(())
}

/// 停止并删除服务，然后还原所有修改。服务已经不存在时只做还原（用于重试）。
/// 有修改没能还原时返回错误：调用方据此保留修改记录，之后可以再次运行重试。
pub fn uninstall() -> anyhow::Result<()> {
    remove_service()?;
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
    println!("Poise 服务已卸载，所有修改都已还原。");
    Ok(())
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
