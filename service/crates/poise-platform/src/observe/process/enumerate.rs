//! 一次系统调用拿到所有进程的动态数据，不需要逐个打开进程。

use std::ffi::c_void;

use poise_core::model::{PriorityClass, ProcessKey, ProcessMetrics};
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
use windows::Win32::Foundation::{STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING};

const SYSTEM_PROCESS_INFORMATION_CLASS: SYSTEM_INFORMATION_CLASS = SYSTEM_INFORMATION_CLASS(5);

/// SYSTEM_PROCESS_INFORMATION 的完整布局（x64）。
#[repr(C)]
struct SystemProcessInformation {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UNICODE_STRING,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

/// 从系统进程表里读出的一行。
#[derive(Debug, Clone)]
pub struct RawProcess {
    pub key: ProcessKey,
    pub parent_pid: u32,
    pub name: String,
    pub session_id: u32,
    pub base_priority: PriorityClass,
    pub metrics: ProcessMetrics,
}

/// 复用缓冲区的进程表读取器。
#[derive(Default)]
pub struct ProcessTable {
    buf: Vec<u64>,
}

impl ProcessTable {
    pub fn read(&mut self) -> windows::core::Result<Vec<RawProcess>> {
        if self.buf.is_empty() {
            self.buf = vec![0; 128 * 1024];
        }
        loop {
            let mut needed = 0u32;
            let bytes = (self.buf.len() * 8) as u32;
            // SAFETY: 缓冲区大小与传入长度一致；返回的结构在缓冲区内部。
            let status = unsafe {
                NtQuerySystemInformation(
                    SYSTEM_PROCESS_INFORMATION_CLASS,
                    self.buf.as_mut_ptr() as *mut c_void,
                    bytes,
                    &mut needed,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                let words = (needed as usize).max(bytes as usize * 2) / 8 + 4096;
                self.buf = vec![0; words];
                continue;
            }
            status.ok()?;
            break;
        }
        Ok(self.parse())
    }

    fn parse(&self) -> Vec<RawProcess> {
        let base = self.buf.as_ptr() as *const u8;
        let mut out = Vec::with_capacity(512);
        let mut offset = 0usize;
        loop {
            // SAFETY: 偏移量来自系统返回的链表，始终落在缓冲区内且按 8 字节对齐。
            let info = unsafe { &*(base.add(offset) as *const SystemProcessInformation) };
            out.push(to_raw(info));
            if info.next_entry_offset == 0 {
                break;
            }
            offset += info.next_entry_offset as usize;
        }
        out
    }
}

fn to_raw(info: &SystemProcessInformation) -> RawProcess {
    let pid = info.unique_process_id as u32;
    let name = if info.image_name.Buffer.is_null() {
        if pid == 0 { "idle" } else { "system" }.to_string()
    } else {
        // SAFETY: Length 以字节计，Buffer 指向同一缓冲区内的 UTF-16 数据。
        let s = unsafe {
            std::slice::from_raw_parts(
                info.image_name.Buffer.0,
                info.image_name.Length as usize / 2,
            )
        };
        String::from_utf16_lossy(s).to_lowercase()
    };
    RawProcess {
        key: ProcessKey {
            pid,
            created: info.create_time as u64,
        },
        parent_pid: info.inherited_from_unique_process_id as u32,
        name,
        session_id: info.session_id,
        base_priority: priority_from_base(info.base_priority),
        metrics: ProcessMetrics {
            private_bytes: info.private_page_count as u64,
            memory_bytes: info.working_set_private_size.max(0) as u64,
            handles: info.handle_count,
            threads: info.number_of_threads,
            cpu_time: (info.user_time + info.kernel_time) as u64,
            io_bytes: (info.read_transfer_count
                + info.write_transfer_count
                + info.other_transfer_count) as u64,
        },
    }
}

/// 把进程基础优先级映射回优先级类别。
fn priority_from_base(base: i32) -> PriorityClass {
    match base {
        ..=4 => PriorityClass::Idle,
        5..=6 => PriorityClass::BelowNormal,
        7..=8 => PriorityClass::Normal,
        9..=10 => PriorityClass::AboveNormal,
        11..=15 => PriorityClass::High,
        _ => PriorityClass::Realtime,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_own_process() {
        let mut t = ProcessTable::default();
        let all = t.read().expect("NtQuerySystemInformation");
        let me = std::process::id();
        let mine = all.iter().find(|p| p.key.pid == me).expect("self in table");
        assert!(mine.name.ends_with(".exe"));
        assert!(mine.metrics.private_bytes > 0);
        assert!(mine.metrics.threads > 0);
        assert!(all.len() > 20);
    }

    #[test]
    fn base_priority_mapping() {
        assert_eq!(priority_from_base(8), PriorityClass::Normal);
        assert_eq!(priority_from_base(13), PriorityClass::High);
        assert_eq!(priority_from_base(4), PriorityClass::Idle);
    }
}
