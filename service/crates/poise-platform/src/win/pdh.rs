//! 性能计数器（PDH，任务管理器用的同一套数据）：打开查询、加计数器、采集、读格式化后的值。只读。
//!
//! 计数器的值是两次采集之间的平均（处理器性能、功耗、显卡占用都是这样），所以先采一次作为起点，下次
//! 采集后再读。带通配实例的计数器（`\Processor Information(*)\...`）每次采集时重新展开实例。

use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PDH_NO_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows::core::PCWSTR;

use super::wide::WideString;

/// 一个打开的查询，丢弃时关闭（连同其中的计数器）。
pub struct Query {
    handle: PDH_HQUERY,
}

// SAFETY: PDH 句柄可以在任意线程使用；查询由持有者独占，不会并发访问。
unsafe impl Send for Query {}

/// 查询里的一个计数器，只在它所属的 [`Query`] 存活期间有效。
#[derive(Debug, Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

/// 一次采集的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collected {
    Data,
    /// 通配的计数器眼下一个实例都没有（例如这个进程没在用显卡）。
    NoData,
    Failed,
}

impl Query {
    pub fn open() -> Option<Self> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: 输出到本地变量。
        (unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) } == 0).then_some(Self { handle })
    }

    /// 加一个计数器（英文路径，不随系统语言变）；这台机器没有这个计数器时为 None。
    pub fn add(&self, path: &str) -> Option<Counter> {
        let path = WideString::new(path);
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: 句柄在 Query 存活期间有效，输出到本地变量。
        let status =
            unsafe { PdhAddEnglishCounterW(self.handle, path.as_pcwstr(), 0, &mut counter) };
        (status == 0).then_some(Counter(counter))
    }

    /// 采一次样：之后读到的是和上一次采集之间的平均。
    pub fn collect(&self) -> Collected {
        // SAFETY: 句柄在 Query 存活期间有效。
        match unsafe { PdhCollectQueryData(self.handle) } {
            0 => Collected::Data,
            PDH_NO_DATA => Collected::NoData,
            _ => Collected::Failed,
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

impl Counter {
    /// 单个实例的值；这次没有有效数据时为 None。
    pub fn value(self) -> Option<f64> {
        let mut v = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: 输出到本地变量。
        let status = unsafe { PdhGetFormattedCounterValue(self.0, PDH_FMT_DOUBLE, None, &mut v) };
        // SAFETY: 按 PDH_FMT_DOUBLE 取值（结构先清零过，没写入时读到的是 0，随后丢弃）。
        let value = unsafe { v.Anonymous.doubleValue };
        (status == 0 && valid(v.CStatus)).then_some(value)
    }

    /// 通配计数器每个实例的有效值（实例名，值）；一个实例都没有时为空，读取出错时为 None。
    pub fn instances(self) -> Option<Vec<(String, f64)>> {
        let (mut bytes, mut count) = (0u32, 0u32);
        // SAFETY: 先询问需要的缓冲区大小。
        let status = unsafe {
            PdhGetFormattedCounterArrayW(self.0, PDH_FMT_DOUBLE, &mut bytes, &mut count, None)
        };
        match status {
            PDH_MORE_DATA if bytes > 0 => {}
            PDH_NO_DATA => return Some(Vec::new()),
            0 if count == 0 => return Some(Vec::new()),
            _ => return None,
        }
        // 按 8 字节对齐分配（结构里有指针和 double），实例名的字符串也写在这块缓冲区里。
        let mut buf = vec![0u64; (bytes as usize).div_ceil(8)];
        let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        // SAFETY: 缓冲区不小于系统要求的大小。
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                self.0,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                Some(items),
            )
        };
        if status != 0 {
            return None;
        }
        // SAFETY: 系统在缓冲区开头写入 count 个结构，名称指向同一块缓冲区里 NUL 结尾的字符串。
        let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
        Some(
            items
                .iter()
                .filter(|i| valid(i.FmtValue.CStatus))
                .filter_map(|i| {
                    // SAFETY: 同上；按 PDH_FMT_DOUBLE 取值。
                    let name = unsafe { i.szName.to_string() }.ok()?;
                    Some((name, unsafe { i.FmtValue.Anonymous.doubleValue }))
                })
                .collect(),
        )
    }
}

fn valid(status: u32) -> bool {
    matches!(status, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_single_and_wildcard_counters() {
        let query = Query::open().expect("打不开性能计数器查询");
        let total = query
            .add(r"\Processor Information(_Total)\% Processor Time")
            .unwrap();
        let cores = query
            .add(r"\Processor Information(*)\% Processor Time")
            .unwrap();
        assert!(query.add(r"\No Such Counter Set(*)\Nothing").is_none());
        assert_eq!(query.collect(), Collected::Data);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(query.collect(), Collected::Data);
        assert!(total.value().is_some_and(|v| (0.0..=100.0).contains(&v)));
        let names = cores.instances().unwrap();
        assert!(names.iter().any(|(n, _)| n == "0,0"), "{names:?}");
    }
}
