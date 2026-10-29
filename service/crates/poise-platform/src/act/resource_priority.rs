//! 进程的内存与磁盘读写优先级，一起调整。
//!
//! 只在"正常"和"低"之间切换：程序自己调得更低的（比如进入了后台模式，两项都是"很低"）不动，
//! 还原时也不会把它调回正常。

use std::ffi::c_void;

use poise_core::action::{ExecError, ResourcePriority};
use poise_core::model::Pid;
use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{HANDLE, NTSTATUS};
use windows::Win32::System::Threading::{
    GetProcessInformation, MEMORY_PRIORITY, MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_LOW,
    MEMORY_PRIORITY_NORMAL, PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION,
    ProcessMemoryPriority, SetProcessInformation,
};

use crate::win::handle::{map_error, open_process};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSetInformationProcess(
        process: HANDLE,
        class: PROCESSINFOCLASS,
        info: *const c_void,
        len: u32,
    ) -> NTSTATUS;
}

/// 磁盘读写优先级（`IO_PRIORITY_HINT`）：0 很低、1 低、2 正常。
const PROCESS_IO_PRIORITY: PROCESSINFOCLASS = PROCESSINFOCLASS(33);
const IO_PRIORITY_LOW: u32 = 1;
const IO_PRIORITY_NORMAL: u32 = 2;

pub fn set(pid: Pid, priority: ResourcePriority) -> Result<(), ExecError> {
    let h = open_process(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION)?;
    let ((memory_from, memory_to), (io_from, io_to)) = match priority {
        ResourcePriority::Low => (
            (MEMORY_PRIORITY_NORMAL, MEMORY_PRIORITY_LOW),
            (IO_PRIORITY_NORMAL, IO_PRIORITY_LOW),
        ),
        ResourcePriority::Default => (
            (MEMORY_PRIORITY_LOW, MEMORY_PRIORITY_NORMAL),
            (IO_PRIORITY_LOW, IO_PRIORITY_NORMAL),
        ),
    };
    // 读不到当前值时照样设置。
    let memory = match memory_priority(h.raw()) {
        Some(current) if current != memory_from => Ok(()),
        _ => set_memory_priority(h.raw(), memory_to),
    };
    let io = match io_priority(h.raw()) {
        Some(current) if current != io_from => Ok(()),
        _ => set_io_priority(h.raw(), io_to),
    };
    memory.and(io)
}

fn memory_priority(h: HANDLE) -> Option<MEMORY_PRIORITY> {
    let mut info = MEMORY_PRIORITY_INFORMATION::default();
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        GetProcessInformation(
            h,
            ProcessMemoryPriority,
            &mut info as *mut _ as *mut c_void,
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
    .ok()
    .map(|()| info.MemoryPriority)
}

fn set_memory_priority(h: HANDLE, priority: MEMORY_PRIORITY) -> Result<(), ExecError> {
    let info = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: priority,
    };
    // SAFETY: 结构体大小与传入长度一致。
    unsafe {
        SetProcessInformation(
            h,
            ProcessMemoryPriority,
            &info as *const _ as *const c_void,
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
    .map_err(|e| map_error(&e))
}

fn io_priority(h: HANDLE) -> Option<u32> {
    let mut current = 0u32;
    // SAFETY: 缓冲区大小与传入长度一致。
    unsafe {
        NtQueryInformationProcess(
            h,
            PROCESS_IO_PRIORITY,
            &mut current as *mut _ as *mut c_void,
            size_of::<u32>() as u32,
            std::ptr::null_mut(),
        )
    }
    .is_ok()
    .then_some(current)
}

fn set_io_priority(h: HANDLE, priority: u32) -> Result<(), ExecError> {
    // SAFETY: 缓冲区大小与传入长度一致。
    unsafe {
        NtSetInformationProcess(
            h,
            PROCESS_IO_PRIORITY,
            &priority as *const _ as *const c_void,
            size_of::<u32>() as u32,
        )
    }
    .ok()
    .map_err(|e| map_error(&e))
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command};

    use super::*;

    /// 只对测试自己启动的子进程操作。
    struct Sandbox(Child);

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn read(pid: Pid) -> (MEMORY_PRIORITY, u32) {
        let h = open_process(pid, PROCESS_QUERY_INFORMATION).unwrap();
        (
            memory_priority(h.raw()).unwrap(),
            io_priority(h.raw()).unwrap(),
        )
    }

    #[test]
    fn lowers_and_restores_both_and_leaves_lower_settings_alone() {
        let child = Sandbox(
            Command::new("cmd.exe")
                .args(["/c", "ping", "-n", "30", "127.0.0.1", ">nul"])
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        assert_eq!(read(pid), (MEMORY_PRIORITY_NORMAL, IO_PRIORITY_NORMAL));
        set(pid, ResourcePriority::Low).unwrap();
        assert_eq!(read(pid), (MEMORY_PRIORITY_LOW, IO_PRIORITY_LOW));
        set(pid, ResourcePriority::Default).unwrap();
        assert_eq!(read(pid), (MEMORY_PRIORITY_NORMAL, IO_PRIORITY_NORMAL));

        // 程序自己调得更低的：还原时不动。
        let h = open_process(pid, PROCESS_SET_INFORMATION).unwrap();
        set_memory_priority(h.raw(), MEMORY_PRIORITY(1)).unwrap();
        set_io_priority(h.raw(), 0).unwrap();
        set(pid, ResourcePriority::Default).unwrap();
        assert_eq!(read(pid), (MEMORY_PRIORITY(1), 0));
    }
}
