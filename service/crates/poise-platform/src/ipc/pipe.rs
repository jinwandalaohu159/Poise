//! 命名管道服务端。
//!
//! - 拒绝远程客户端（PIPE_REJECT_REMOTE_CLIENTS）；
//! - 第一个实例使用 FILE_FLAG_FIRST_PIPE_INSTANCE，名字被别人抢占时直接失败，防止"管道抢注"；
//! - 服务模式下用安全描述符限制只有本机交互用户可以连接。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use poise_core::model::Pid;
use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX, ReadFile,
    WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};

use crate::win::handle::OwnedHandle;
use crate::win::security::SecurityDescriptor;
use crate::win::wide::WideString;

const BUFFER: u32 = 64 * 1024;

pub struct PipeServer {
    name: String,
    sd: Option<SecurityDescriptor>,
    first: AtomicBool,
}

// SAFETY: 安全描述符只读共享；创建实例的调用是线程安全的。
unsafe impl Send for PipeServer {}
unsafe impl Sync for PipeServer {}

impl PipeServer {
    /// `sddl` 为 None 时使用默认安全描述符（只有当前用户和管理员可连）。
    pub fn new(name: &str, sddl: Option<&str>) -> windows::core::Result<Self> {
        Ok(Self {
            name: name.to_string(),
            sd: sddl.map(SecurityDescriptor::from_sddl).transpose()?,
            first: AtomicBool::new(true),
        })
    }

    /// 创建一个新实例并阻塞等待客户端连接。
    pub fn accept(&self) -> windows::core::Result<PipeConnection> {
        let name = WideString::new(&self.name);
        // 第一个实例必须是自己建的（别的程序抢先建了同名管道时失败）。没建成时下次还要这样建，
        // 不能退而加入别人的管道。
        let first = self.first.load(Ordering::SeqCst);
        let mut open_mode: FILE_FLAGS_AND_ATTRIBUTES = PIPE_ACCESS_DUPLEX;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let sa = self.sd.as_ref().map(SecurityDescriptor::attributes);
        // SAFETY: 名称与安全属性在调用期间有效。
        let h = unsafe {
            CreateNamedPipeW(
                name.as_pcwstr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                sa.as_ref().map(|s| s as *const _),
            )
        };
        if h.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }
        self.first.store(false, Ordering::SeqCst);
        let handle = OwnedHandle::from_raw(h);
        // SAFETY: 句柄有效，同步模式。
        if let Err(e) = unsafe { ConnectNamedPipe(handle.raw(), None) }
            && WIN32_ERROR::from_error(&e) != Some(ERROR_PIPE_CONNECTED)
        {
            return Err(e);
        }
        Ok(PipeConnection { handle })
    }
}

pub struct PipeConnection {
    handle: OwnedHandle,
}

impl PipeConnection {
    pub fn client_pid(&self) -> Option<Pid> {
        let mut pid = 0u32;
        // SAFETY: 输出到本地变量。
        unsafe { GetNamedPipeClientProcessId(self.handle.raw(), &mut pid) }.ok()?;
        Some(pid)
    }
}

impl Read for PipeConnection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n = 0u32;
        // SAFETY: 缓冲区由调用方提供，长度与切片一致。
        match unsafe { ReadFile(self.handle.raw(), Some(buf), Some(&mut n), None) } {
            Ok(()) => Ok(n as usize),
            Err(e) if WIN32_ERROR::from_error(&e) == Some(ERROR_BROKEN_PIPE) => Ok(0),
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

impl Write for PipeConnection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut n = 0u32;
        // SAFETY: 同上。
        unsafe { WriteFile(self.handle.raw(), Some(buf), Some(&mut n), None) }
            .map_err(io::Error::other)?;
        Ok(n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        // SAFETY: 断开后由 OwnedHandle 关闭句柄。
        unsafe {
            let _ = DisconnectNamedPipe(self.handle.raw());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};

    use super::*;

    #[test]
    fn echo_round_trip_and_client_pid() {
        let name = format!(r"\\.\pipe\PoiseTest-{}", std::process::id());
        let server = PipeServer::new(&name, None).unwrap();
        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            let mut f = loop {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&client_name)
                {
                    Ok(f) => break f,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            };
            f.write_all(b"hello\n").unwrap();
            let mut line = String::new();
            BufReader::new(f).read_line(&mut line).unwrap();
            line
        });
        let conn = server.accept().unwrap();
        assert_eq!(conn.client_pid(), Some(std::process::id()));
        let mut reader = BufReader::new(conn);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        reader.get_mut().write_all(line.as_bytes()).unwrap();
        assert_eq!(client.join().unwrap(), "hello\n");
    }

    #[test]
    fn second_server_cannot_squat_the_same_name() {
        let name = format!(r"\\.\pipe\PoiseSquat-{}", std::process::id());
        let a = PipeServer::new(&name, None).unwrap();
        let n2 = name.clone();
        // 第一个实例在后台等待连接。
        let t = std::thread::spawn(move || a.accept().map(|_| ()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let b = PipeServer::new(&n2, None).unwrap();
        assert!(b.accept().is_err());
        // 连接一次让后台线程结束。
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name);
        let _ = t.join();
    }
}
