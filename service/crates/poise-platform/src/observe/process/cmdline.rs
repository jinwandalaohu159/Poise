//! 读取另一个进程的启动命令行，用于"记录后重启"。

use std::ffi::c_void;

use poise_core::action::ExecError;
use poise_core::model::Pid;
use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::UNICODE_STRING;
use windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;

use crate::win::handle::{image_path_of, open_process};

/// ProcessCommandLineInformation（Windows 8.1+）。
const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);

/// 重启一个进程需要的全部信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub image_path: String,
    pub command_line: String,
    pub working_dir: String,
}

pub fn capture(pid: Pid) -> Result<LaunchSpec, ExecError> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let image_path = image_path_of(&h).ok_or(ExecError::AccessDenied)?;
    // 一般的命令行放得下；放不下（最长 32767 个字符）时按系统报告的大小再读一次。
    let mut buf = vec![0u64; 512];
    for _ in 0..2 {
        let mut len = 0u32;
        // SAFETY: 缓冲区长度如实传入；返回的 UNICODE_STRING 指向缓冲区内部。
        let status = unsafe {
            NtQueryInformationProcess(
                h.raw(),
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr() as *mut c_void,
                (buf.len() * 8) as u32,
                &mut len,
            )
        };
        if status.is_ok() {
            return Ok(spec(&buf, image_path));
        }
        if len as usize <= buf.len() * 8 {
            break;
        }
        buf = vec![0u64; (len as usize).div_ceil(8)];
    }
    Err(ExecError::AccessDenied)
}

/// 由读到的命令行（缓冲区开头的 UNICODE_STRING）组装重启信息。
fn spec(buf: &[u64], image_path: String) -> LaunchSpec {
    // SAFETY: 缓冲区开头就是 UNICODE_STRING。
    let us = unsafe { &*(buf.as_ptr() as *const UNICODE_STRING) };
    let command_line = if us.Buffer.is_null() || us.Length == 0 {
        format!("\"{image_path}\"")
    } else {
        // SAFETY: Length 字节有效。
        let s = unsafe { std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2) };
        String::from_utf16_lossy(s)
    };
    let working_dir = std::path::Path::new(&image_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    LaunchSpec {
        image_path,
        command_line,
        working_dir,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn captures_own_command_line() {
        let spec = super::capture(std::process::id()).unwrap();
        assert!(spec.image_path.to_lowercase().ends_with(".exe"));
        assert!(!spec.command_line.is_empty());
    }
}
