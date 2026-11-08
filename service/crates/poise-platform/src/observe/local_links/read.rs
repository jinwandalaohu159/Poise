//! 读系统的 TCP 连接表（IPv4、IPv6，带所属进程号）。只读，不打开任何进程。

use poise_core::model::Pid;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

/// 连接表里的状态：监听、已建立。
pub(super) const LISTEN: u32 = 2;
pub(super) const ESTABLISHED: u32 = 5;
/// 连接表最多看这么多行（异常多时不全看，只是少认出几个）。
const MAX_ROWS: usize = 20_000;

/// 一端：地址（16 字节，IPv4 映射到 IPv6 的写法）和端口。
pub(super) type Endpoint = ([u8; 16], u16);

/// 一行连接。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Row {
    pub state: u32,
    pub local: Endpoint,
    pub remote: Endpoint,
    pub pid: Pid,
}

/// 两种地址族的连接表合在一起（只要监听和已建立的）；读不到的那种为空。
pub(super) fn read_all() -> Vec<Row> {
    let mut rows = read_rows(AF_INET.0 as u32);
    rows.extend(read_rows(AF_INET6.0 as u32));
    rows
}

/// 读一种地址族的连接表；读不到时为空。
fn read_rows(family: u32) -> Vec<Row> {
    let mut len = 0u32;
    // SAFETY: 先询问所需大小。
    unsafe {
        GetExtendedTcpTable(None, &mut len, false, family, TCP_TABLE_OWNER_PID_ALL, 0);
    }
    if len == 0 {
        return Vec::new();
    }
    // 两次调用之间连接可能变多：多留一些。
    len += 4096;
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节，按 8 字节对齐。
    let status = unsafe {
        GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut len,
            false,
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if status != 0 {
        return Vec::new();
    }
    let base = buf.as_ptr() as *const u8;
    // SAFETY: 表以 4 字节的行数开头；IPv4 的行紧接其后，IPv6 的行按 4 字节对齐也紧接其后。
    let count = (unsafe { *(base as *const u32) } as usize).min(MAX_ROWS);
    let rows_at = unsafe { base.add(4) };
    let capacity = (len as usize).saturating_sub(4);
    if family == AF_INET.0 as u32 {
        let n = count.min(capacity / size_of::<MIB_TCPROW_OWNER_PID>());
        // SAFETY: n 行都在缓冲区内。
        let rows = unsafe { std::slice::from_raw_parts(rows_at as *const MIB_TCPROW_OWNER_PID, n) };
        rows.iter()
            .filter(|r| r.dwState == LISTEN || r.dwState == ESTABLISHED)
            .map(|r| Row {
                state: r.dwState,
                local: (v4(r.dwLocalAddr), port(r.dwLocalPort)),
                remote: (v4(r.dwRemoteAddr), port(r.dwRemotePort)),
                pid: r.dwOwningPid,
            })
            .collect()
    } else {
        let n = count.min(capacity / size_of::<MIB_TCP6ROW_OWNER_PID>());
        // SAFETY: 同上。
        let rows =
            unsafe { std::slice::from_raw_parts(rows_at as *const MIB_TCP6ROW_OWNER_PID, n) };
        rows.iter()
            .filter(|r| r.dwState == LISTEN || r.dwState == ESTABLISHED)
            .map(|r| Row {
                state: r.dwState,
                local: (r.ucLocalAddr, port(r.dwLocalPort)),
                remote: (r.ucRemoteAddr, port(r.dwRemotePort)),
                pid: r.dwOwningPid,
            })
            .collect()
    }
}

/// IPv4 地址（网络字节序）写成映射到 IPv6 的 16 字节。
fn v4(addr: u32) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[10] = 0xff;
    out[11] = 0xff;
    out[12..].copy_from_slice(&addr.to_ne_bytes());
    out
}

/// 端口：低 16 位，网络字节序。
fn port(raw: u32) -> u16 {
    u16::from_be((raw & 0xffff) as u16)
}
