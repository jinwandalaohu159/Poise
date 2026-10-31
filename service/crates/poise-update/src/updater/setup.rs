//! 安装与启动：以管理员身份静默运行安装程序并等它结束，然后等服务启动、启动托盘代理。

use std::path::Path;
use std::time::{Duration, Instant};

use poise_platform::win::handle::OwnedHandle;
use poise_platform::win::wide::WideString;
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_HANDLE,
    SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HRESULT, w};

use super::progress::Failure;

/// 安装程序的参数：不显示任何界面、不弹提示框、不重启、不能取消（Inno Setup）。
/// 安装位置沿用上次的选择；安装完成后不启动托盘代理（由本程序以用户身份启动）。
const INSTALLER_ARGS: &str = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP-";

const SERVICE_NAME: &str = "Poise";
/// 等服务启动的最长时间。
const SERVICE_TIMEOUT: Duration = Duration::from_secs(60);

/// 等安装程序最多这么久（正常不到一分钟；要关闭正在运行的 Poise、复制文件、注册服务）。
const INSTALL_TIMEOUT_MS: u32 = 10 * 60_000;

/// 以管理员身份运行安装程序（系统会弹出权限提示）并等它结束。
pub fn run_installer(installer: &Path, owner: isize) -> Result<(), Failure> {
    let file = WideString::new(&installer.to_string_lossy());
    // 安装日志写在安装包旁边（用户自己的目录里）：静默安装失败时只有退出码，原因要看它。
    let log = installer.with_extension("log");
    let params = WideString::new(&format!(
        "{INSTALLER_ARGS} \"/LOG={}\"",
        log.to_string_lossy()
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: HWND(owner as *mut _),
        lpVerb: w!("runas"),
        lpFile: file.as_pcwstr(),
        lpParameters: params.as_pcwstr(),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: 结构体里的字符串在调用期间有效。
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
            log::info!("用户拒绝了管理员权限");
            Failure::Declined
        } else {
            log::warn!("启动安装程序失败：{e}");
            Failure::Launch(e.message())
        });
    }
    let process = OwnedHandle::from_raw(info.hProcess);
    // SAFETY: 等待自己启动的进程。安装程序卡住时不一直等下去（进度窗口会一直停在"安装中"）。
    if unsafe { WaitForSingleObject(process.raw(), INSTALL_TIMEOUT_MS) } != WAIT_OBJECT_0 {
        log::warn!("安装程序 {} 分钟内没有结束", INSTALL_TIMEOUT_MS / 60_000);
        return Err(Failure::Unfinished("安装程序没有及时结束".into()));
    }
    let mut code = 0u32;
    // SAFETY: 进程句柄有效，输出到本地变量。
    unsafe { GetExitCodeProcess(process.raw(), &mut code) }
        .map_err(|e| Failure::Unfinished(e.message()))?;
    log::info!("安装程序结束，退出码 {code}");
    if code == 0 {
        Ok(())
    } else {
        log::warn!("安装没有完成，安装日志：{}", log.display());
        Err(Failure::Install(code))
    }
}

/// 等 Poise 服务进入运行状态。`progress` 收到已经等待的比例（0～1，按超时时间算）。
pub fn wait_for_service(progress: impl Fn(f32)) -> Result<(), Failure> {
    let start = Instant::now();
    loop {
        if service_running() {
            return Ok(());
        }
        let waited = start.elapsed();
        if waited >= SERVICE_TIMEOUT {
            log::warn!("等待服务启动超时");
            return Err(Failure::Start);
        }
        progress(waited.as_secs_f32() / SERVICE_TIMEOUT.as_secs_f32());
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// 以当前用户的身份启动托盘代理（它只允许一个实例，已经在运行时新启动的会自行退出）。
pub fn start_agent(dir: &Path) {
    let agent = dir.join("poise-agent.exe");
    match std::process::Command::new(&agent).current_dir(dir).spawn() {
        Ok(_) => log::info!("已启动托盘代理"),
        Err(e) => log::warn!("启动托盘代理失败：{e}"),
    }
}

fn service_running() -> bool {
    let name = WideString::new(SERVICE_NAME);
    // SAFETY: 只申请查询权限，句柄在本函数内关闭。
    unsafe {
        let Ok(manager) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else {
            return false;
        };
        let running = OpenServiceW(manager, name.as_pcwstr(), SERVICE_QUERY_STATUS)
            .ok()
            .is_some_and(|service| {
                let mut status = SERVICE_STATUS::default();
                let ok = QueryServiceStatus(service, &mut status).is_ok();
                close(service);
                ok && status.dwCurrentState == SERVICE_RUNNING
            });
        close(manager);
        running
    }
}

fn close(handle: SC_HANDLE) {
    // SAFETY: 句柄由调用方打开，只关闭一次。
    let _ = unsafe { CloseServiceHandle(handle) };
}
