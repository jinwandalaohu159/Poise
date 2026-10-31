//! 直接问显卡内核（D3DKMT）：一个进程在各块显卡的 3D 引擎上累计运行了多久。任务管理器"GPU"列的同一来源，
//! 但不经过性能计数器——性能计数器打开查询时要枚举全系统所有进程的所有显卡引擎，一次要 0.5 秒处理器时间；
//! 这里每次读数只是几次系统调用。
//!
//! 显卡和它们的 3D 引擎在创建时枚举（调用方每开始测一个新进程时重新创建，显卡变了也能跟上）。

use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTERINFO, D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_NODEMETADATA,
    D3DKMT_QUERY_DEVICE_IDS, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE,
    D3DKMT_QUERYSTATISTICS_PROCESS_NODE, D3DKMTCloseAdapter, D3DKMTEnumAdapters2,
    D3DKMTQueryAdapterInfo, D3DKMTQueryStatistics, DXGK_ENGINE_TYPE_3D, KMTQAITYPE_NODEMETADATA,
    KMTQAITYPE_PHYSICALADAPTERDEVICEIDS,
};
use windows::Win32::Foundation::{HANDLE, LUID, STATUS_INVALID_PARAMETER};

/// 一块显卡，以及它上面的 3D 引擎编号。
struct Adapter {
    luid: LUID,
    nodes_3d: Vec<u32>,
}

pub(super) struct KernelStats {
    adapters: Vec<Adapter>,
}

impl KernelStats {
    /// 枚举显卡和 3D 引擎；系统不支持或一块带 3D 引擎的显卡都没有时为 None。
    pub fn open() -> Option<Self> {
        let adapters: Vec<Adapter> = enum_adapters()
            .into_iter()
            .filter_map(|info| {
                let nodes_3d = nodes_3d(&info);
                close_adapter(info.hAdapter);
                (!nodes_3d.is_empty()).then_some(Adapter {
                    luid: info.AdapterLuid,
                    nodes_3d,
                })
            })
            .collect();
        (!adapters.is_empty()).then_some(Self { adapters })
    }

    /// 进程在所有 3D 引擎上累计的运行时间（100 纳秒）。`process` 要有完整的查询信息权限
    /// （`PROCESS_QUERY_INFORMATION`）：只有受限查询权限时，系统对每个引擎都回答"参数错误"，
    /// 和"没用过这块显卡"分不开。查询失败（显卡变了）时为 None。
    pub fn running_3d(&self, process: HANDLE) -> Option<u64> {
        self.query_3d(process).map(|(total, _)| total)
    }

    /// 开始测一个进程时用：同 [`KernelStats::running_3d`]，但每个引擎都回答"参数错误"时也为 None——
    /// 前台全屏的程序不会一块显卡都没用过，这时多半是系统不肯回答这个进程（权限不够），
    /// 一直读到 0 就永远认不出游戏，调用方改用性能计数器。
    pub fn first_running_3d(&self, process: HANDLE) -> Option<u64> {
        match self.query_3d(process)? {
            (total, true) => Some(total),
            (_, false) => None,
        }
    }

    /// 累计运行时间，以及有没有哪个引擎给出了回答（不是"参数错误"）。
    fn query_3d(&self, process: HANDLE) -> Option<(u64, bool)> {
        let mut answered = false;
        let mut total = 0u64;
        for adapter in &self.adapters {
            for &node in &adapter.nodes_3d {
                let mut q = D3DKMT_QUERYSTATISTICS {
                    Type: D3DKMT_QUERYSTATISTICS_PROCESS_NODE,
                    AdapterLuid: adapter.luid,
                    hProcess: process,
                    ..Default::default()
                };
                q.Anonymous.QueryProcessNode.NodeId = node;
                // SAFETY: 结构体按 Type 填好；结果写在结构体内部（所以按可写的指针传，
                // 否则编译器可以认为它没变）。
                let status = unsafe { D3DKMTQueryStatistics(std::ptr::from_mut(&mut q)) };
                match status {
                    // 进程还没在这块显卡上用过图形功能：运行时间为 0。
                    STATUS_INVALID_PARAMETER => {}
                    s if s.is_ok() => {
                        // SAFETY: 查询成功，结果按 Type 填在 ProcessNodeInformation 里。
                        let running = unsafe { q.QueryResult.ProcessNodeInformation.RunningTime };
                        total += running.max(0) as u64;
                        answered = true;
                    }
                    _ => return None,
                }
            }
        }
        Some((total, answered))
    }
}

/// 某个厂商（PCI 厂商编号）的显卡及其引擎数量。
pub(super) fn adapters_of(vendor: u32) -> Vec<(LUID, u32)> {
    enum_adapters()
        .into_iter()
        .filter_map(|info| {
            let found =
                (vendor_of(&info) == Some(vendor)).then(|| (info.AdapterLuid, node_count(&info)));
            close_adapter(info.hAdapter);
            found
        })
        .collect()
}

/// 一块显卡上一个引擎从开机以来一共运行了多久（100 纳秒）。只读显卡内核里的统计，不碰显卡本身。
pub(super) fn node_running(luid: LUID, node: u32) -> Option<u64> {
    let mut q = D3DKMT_QUERYSTATISTICS {
        Type: D3DKMT_QUERYSTATISTICS_NODE,
        AdapterLuid: luid,
        ..Default::default()
    };
    q.Anonymous.QueryNode.NodeId = node;
    // SAFETY: 结构体按 Type 填好；结果写在结构体内部（按可写的指针传）。
    unsafe {
        if D3DKMTQueryStatistics(std::ptr::from_mut(&mut q)).is_err() {
            return None;
        }
        Some(
            q.QueryResult
                .NodeInformation
                .GlobalInformation
                .RunningTime
                .max(0) as u64,
        )
    }
}

/// 显卡的 PCI 厂商编号（Windows 10 1803 起支持）。
fn vendor_of(info: &D3DKMT_ADAPTERINFO) -> Option<u32> {
    let mut ids = D3DKMT_QUERY_DEVICE_IDS::default();
    let mut ask = D3DKMT_QUERYADAPTERINFO {
        hAdapter: info.hAdapter,
        Type: KMTQAITYPE_PHYSICALADAPTERDEVICEIDS,
        pPrivateDriverData: &mut ids as *mut _ as *mut core::ffi::c_void,
        PrivateDriverDataSize: size_of::<D3DKMT_QUERY_DEVICE_IDS>() as u32,
    };
    // SAFETY: 输出缓冲区是本地结构体，大小与传入一致。
    let ok = unsafe { D3DKMTQueryAdapterInfo(&mut ask) }.is_ok();
    ok.then_some(ids.DeviceIds.VendorID)
}

/// 一块显卡上的引擎数量。
fn node_count(info: &D3DKMT_ADAPTERINFO) -> u32 {
    let mut q = D3DKMT_QUERYSTATISTICS {
        Type: D3DKMT_QUERYSTATISTICS_ADAPTER,
        AdapterLuid: info.AdapterLuid,
        ..Default::default()
    };
    // SAFETY: 结构体按 Type 填好；结果写在结构体内部（按可写的指针传）。
    unsafe {
        if D3DKMTQueryStatistics(std::ptr::from_mut(&mut q)).is_err() {
            return 0;
        }
        q.QueryResult.AdapterInformation.NodeCount
    }
}

/// 系统里的显卡（含打开的句柄，用完要关）。
fn enum_adapters() -> Vec<D3DKMT_ADAPTERINFO> {
    let mut e = D3DKMT_ENUMADAPTERS2::default();
    // SAFETY: 第一次只问数量（pAdapters 为空）。
    if unsafe { D3DKMTEnumAdapters2(&mut e) }.is_err() || e.NumAdapters == 0 {
        return Vec::new();
    }
    let mut list = vec![D3DKMT_ADAPTERINFO::default(); e.NumAdapters as usize];
    e.pAdapters = list.as_mut_ptr();
    // SAFETY: 缓冲区能放下 NumAdapters 个条目。
    if unsafe { D3DKMTEnumAdapters2(&mut e) }.is_err() {
        return Vec::new();
    }
    list.truncate(e.NumAdapters as usize);
    list
}

/// 一块显卡上类型为 3D 的引擎编号（独立显卡通常一个；有的虚拟显示适配器有十几个）。
fn nodes_3d(info: &D3DKMT_ADAPTERINFO) -> Vec<u32> {
    (0..node_count(info))
        .filter(|&node| {
            let mut meta = D3DKMT_NODEMETADATA {
                NodeOrdinalAndAdapterIndex: node,
                ..Default::default()
            };
            let mut ask = D3DKMT_QUERYADAPTERINFO {
                hAdapter: info.hAdapter,
                Type: KMTQAITYPE_NODEMETADATA,
                pPrivateDriverData: &mut meta as *mut _ as *mut core::ffi::c_void,
                PrivateDriverDataSize: size_of::<D3DKMT_NODEMETADATA>() as u32,
            };
            // SAFETY: 输出缓冲区是本地结构体，大小与传入一致。
            let ok = unsafe { D3DKMTQueryAdapterInfo(&mut ask) }.is_ok();
            // 结构体是紧凑排列的，字段先拷出来再比较。
            let engine = meta.NodeData.EngineType;
            ok && engine == DXGK_ENGINE_TYPE_3D
        })
        .collect()
}

fn close_adapter(handle: u32) {
    // SAFETY: 关闭枚举时打开的显卡句柄。
    let _ = unsafe { D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: handle }) };
}
