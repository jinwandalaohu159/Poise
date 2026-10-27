//! 以用户身份重新启动一个程序。
//!
//! 服务以 SYSTEM 运行时，必须拿用户会话的令牌启动，绝不以 SYSTEM 身份启动用户程序。

use std::ffi::c_void;

use poise_core::action::ExecError;
use poise_core::model::Pid;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::WTSQueryUserToken;
use windows::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::core::PWSTR;

use crate::observe::process::cmdline::LaunchSpec;
use crate::win::handle::{OwnedHandle, map_error};
use crate::win::wide::WideString;

pub trait Launcher: Send + Sync {
    fn launch(&self, spec: &LaunchSpec, session_id: u32) -> Result<Pid, ExecError>;
}

/// 控制台（开发）模式：Poise 本身就以当前用户运行，直接启动即可。
pub struct DirectLauncher;

/// 服务模式：取该会话用户的令牌，在用户桌面上启动。
pub struct UserSessionLauncher;

impl Launcher for DirectLauncher {
    fn launch(&self, spec: &LaunchSpec, _session_id: u32) -> Result<Pid, ExecError> {
        let app = WideString::new(&spec.image_path);
        let mut cmd = WideString::new(&spec.command_line);
        let cwd = WideString::new(&spec.working_dir);
        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        // SAFETY: 所有字符串以 NUL 结尾并在调用期间有效；指定程序路径避免搜索路径劫持。
        unsafe {
            CreateProcessW(
                app.as_pcwstr(),
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NEW_PROCESS_GROUP,
                None,
                cwd.as_pcwstr(),
                &si,
                &mut pi,
            )
        }
        .map_err(|e| map_error(&e))?;
        Ok(finish(pi))
    }
}

impl Launcher for UserSessionLauncher {
    fn launch(&self, spec: &LaunchSpec, session_id: u32) -> Result<Pid, ExecError> {
        let mut token = HANDLE::default();
        // SAFETY: 需要 SYSTEM 权限；输出到本地句柄。
        unsafe { WTSQueryUserToken(session_id, &mut token) }.map_err(|e| map_error(&e))?;
        let token = OwnedHandle::from_raw(token);
        let mut env: *mut c_void = std::ptr::null_mut();
        // SAFETY: 以用户令牌生成环境块，不继承 SYSTEM 的环境。
        unsafe { CreateEnvironmentBlock(&mut env, Some(token.raw()), false) }
            .map_err(|e| map_error(&e))?;
        let app = WideString::new(&spec.image_path);
        let mut cmd = WideString::new(&spec.command_line);
        let cwd = WideString::new(&spec.working_dir);
        let mut desktop = WideString::new(r"winsta0\default");
        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_mut_ptr()),
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        // SAFETY: 参数在调用期间有效；不继承句柄、不提权。
        let result = unsafe {
            CreateProcessAsUserW(
                Some(token.raw()),
                app.as_pcwstr(),
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP,
                Some(env),
                cwd.as_pcwstr(),
                &si,
                &mut pi,
            )
        };
        // SAFETY: 环境块由 CreateEnvironmentBlock 分配。
        unsafe {
            let _ = DestroyEnvironmentBlock(env);
        }
        result.map_err(|e| map_error(&e))?;
        Ok(finish(pi))
    }
}

fn finish(pi: PROCESS_INFORMATION) -> Pid {
    drop(OwnedHandle::from_raw(pi.hThread));
    drop(OwnedHandle::from_raw(pi.hProcess));
    pi.dwProcessId
}
