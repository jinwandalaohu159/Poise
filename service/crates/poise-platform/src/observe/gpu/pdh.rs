//! 后备办法：性能计数器 GPU Engine（同样是任务管理器"GPU"列的来源）。不用打开进程，被反作弊保护、
//! 拿不到进程句柄的游戏也能测；但打开查询要枚举全系统所有进程的所有显卡引擎，一次约 0.5 秒处理器时间，
//! 所以只在直接问显卡内核不行时才用（见 `kmt`）。

use poise_core::model::Pid;
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PDH_NO_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::core::PCWSTR;

use crate::win::wide::WideString;

pub(super) struct Query {
    handle: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

// SAFETY: PDH 句柄可以在任意线程使用；GpuMeter 只在持有引擎锁时被调用，不会并发访问。
unsafe impl Send for Query {}

impl Query {
    pub(super) fn open(pid: Pid) -> Option<Self> {
        // 实例名形如 pid_1234_luid_..._phys_0_eng_0_engtype_3D；通配符在每次采集时重新展开。
        let path = WideString::new(&format!(
            r"\GPU Engine(pid_{pid}_*engtype_3D)\Utilization Percentage"
        ));
        let mut handle = PDH_HQUERY::default();
        // SAFETY: 输出到本地变量；之后出错时 query 被丢弃，查询随之关闭。
        unsafe {
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) != 0 {
                return None;
            }
            let mut query = Self {
                handle,
                counter: PDH_HCOUNTER::default(),
            };
            if PdhAddEnglishCounterW(handle, path.as_pcwstr(), 0, &mut query.counter) != 0 {
                return None;
            }
            query.collect();
            Some(query)
        }
    }

    /// 采一次样，作为下次读数的起点。
    pub(super) fn collect(&self) {
        // SAFETY: 句柄在 Query 存活期间有效。
        unsafe {
            PdhCollectQueryData(self.handle);
        }
    }

    pub(super) fn read(&mut self) -> Option<f32> {
        // SAFETY: 句柄在 Query 存活期间有效；缓冲区按 PDH 返回的大小分配，并按条目类型对齐。
        unsafe {
            match PdhCollectQueryData(self.handle) {
                0 => {}
                // 没有匹配的实例：这个进程没有用显卡。
                PDH_NO_DATA => return Some(0.0),
                _ => return None,
            }
            let (mut size, mut count) = (0u32, 0u32);
            let status = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                None,
            );
            match status {
                PDH_MORE_DATA => {}
                // 没有匹配的实例：这个进程没有用显卡。
                PDH_NO_DATA => return Some(0.0),
                0 if count == 0 => return Some(0.0),
                _ => return None,
            }
            let item = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> =
                Vec::with_capacity((size as usize).div_ceil(item));
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buf.as_mut_ptr()),
            ) != 0
            {
                return None;
            }
            buf.set_len(count as usize);
            let total: f64 = buf
                .iter()
                .filter(|i| {
                    matches!(
                        i.FmtValue.CStatus,
                        PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                    )
                })
                .map(|i| i.FmtValue.Anonymous.doubleValue)
                .sum();
            Some(total.clamp(0.0, 100.0) as f32)
        }
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: 关闭本对象打开的查询（同时释放其中的计数器）。
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}
