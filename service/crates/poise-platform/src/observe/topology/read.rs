//! 读取系统报告的处理器信息（只读）：每个逻辑处理器的 CPU Set（效率等级、所在核心），以及各级缓存由哪些
//! 逻辑处理器共用。

use windows::Win32::System::SystemInformation::{
    CpuSetInformation, GetLogicalProcessorInformationEx, GetSystemCpuSetInformation, RelationCache,
    SYSTEM_CPU_SET_INFORMATION, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

/// 系统报告的一个 CPU Set。
#[derive(Debug, Clone, Copy)]
pub(super) struct CpuSetEntry {
    pub id: u32,
    pub group: u16,
    pub logical_index: u8,
    pub core_index: u8,
    pub efficiency_class: u8,
}

/// 系统报告的一块缓存：级别、容量，以及由哪些逻辑处理器共用（处理器组和组内的位掩码）。
#[derive(Debug, Clone, Copy)]
pub(super) struct CacheEntry {
    pub level: u8,
    pub size: u32,
    pub group: u16,
    pub mask: u64,
}

impl CacheEntry {
    /// 这块缓存是否由这个逻辑处理器使用。
    pub fn covers(&self, s: &CpuSetEntry) -> bool {
        s.group == self.group && s.logical_index < 64 && self.mask & (1u64 << s.logical_index) != 0
    }
}

/// 二级、三级缓存各由哪些逻辑处理器共用。读不到时为空。
pub(super) fn read_caches() -> Vec<CacheEntry> {
    let mut len = 0u32;
    // SAFETY: 先询问所需大小（这次调用按约定返回"缓冲区不足"）。
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationCache, None, &mut len) };
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节，按 8 字节对齐。
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RelationCache,
            Some(buf.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX),
            &mut len,
        )
    };
    if ok.is_err() {
        return Vec::new();
    }
    let mut caches = Vec::new();
    let base = buf.as_ptr() as *const u8;
    let mut offset = 0usize;
    while offset < len as usize {
        // SAFETY: 每个条目以 Size 字段给出自身长度，始终在 len 范围内。
        let entry =
            unsafe { &*(base.add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX) };
        if entry.Relationship == RelationCache {
            // SAFETY: Relationship 为 RelationCache 时联合体的 Cache 成员有效。
            let cache = unsafe { entry.Anonymous.Cache };
            // SAFETY: GroupCount 至少为 1 时第一个掩码有效。
            let mask = unsafe { cache.Anonymous.GroupMask };
            if matches!(cache.Level, 2 | 3) {
                caches.push(CacheEntry {
                    level: cache.Level,
                    size: cache.CacheSize,
                    group: mask.Group,
                    mask: mask.Mask as u64,
                });
            }
        }
        if entry.Size == 0 {
            break;
        }
        offset += entry.Size as usize;
    }
    caches
}

/// 系统报告的所有 CPU Set。读不到时为空。
pub(super) fn read_cpu_sets() -> Vec<CpuSetEntry> {
    let mut len = 0u32;
    // SAFETY: 先询问所需大小。
    let _ =
        unsafe { GetSystemCpuSetInformation(None, 0, &mut len, Some(GetCurrentProcess()), None) };
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: 缓冲区不小于 len 字节。
    let ok = unsafe {
        GetSystemCpuSetInformation(
            Some(buf.as_mut_ptr() as *mut SYSTEM_CPU_SET_INFORMATION),
            len,
            &mut len,
            Some(GetCurrentProcess()),
            None,
        )
    };
    if !ok.as_bool() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let base = buf.as_ptr() as *const u8;
    let mut offset = 0usize;
    while offset < len as usize {
        // SAFETY: 每个条目以 Size 字段给出自身长度，始终在 len 范围内。
        let entry = unsafe { &*(base.add(offset) as *const SYSTEM_CPU_SET_INFORMATION) };
        if entry.Type == CpuSetInformation {
            // SAFETY: Type 为 CpuSetInformation 时联合体的 CpuSet 成员有效。
            let cs = unsafe { entry.Anonymous.CpuSet };
            out.push(CpuSetEntry {
                id: cs.Id,
                group: cs.Group,
                logical_index: cs.LogicalProcessorIndex,
                core_index: cs.CoreIndex,
                efficiency_class: cs.EfficiencyClass,
            });
        }
        if entry.Size == 0 {
            break;
        }
        offset += entry.Size as usize;
    }
    out
}
