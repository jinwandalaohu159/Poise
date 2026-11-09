//! 后备办法：性能计数器 GPU Engine（同样是任务管理器"GPU"列的来源）。不用打开进程，被反作弊保护、
//! 拿不到进程句柄的游戏也能测；但打开查询要枚举全系统所有进程的所有显卡引擎，一次约 0.5 秒处理器时间，
//! 所以只在直接问显卡内核不行时才用（见 `kmt`）。

use poise_core::model::Pid;

use crate::win::pdh::{self, Collected, Counter};

pub(super) struct Query {
    query: pdh::Query,
    counter: Counter,
}

impl Query {
    pub(super) fn open(pid: Pid) -> Option<Self> {
        // 实例名形如 pid_1234_luid_..._phys_0_eng_0_engtype_3D；通配符在每次采集时重新展开。
        let query = pdh::Query::open()?;
        let counter = query.add(&format!(
            r"\GPU Engine(pid_{pid}_*engtype_3D)\Utilization Percentage"
        ))?;
        query.collect();
        Some(Self { query, counter })
    }

    /// 采一次样，作为下次读数的起点。
    pub(super) fn collect(&self) {
        self.query.collect();
    }

    pub(super) fn read(&mut self) -> Option<f32> {
        match self.query.collect() {
            Collected::Data => {}
            // 没有匹配的实例：这个进程没有用显卡。
            Collected::NoData => return Some(0.0),
            Collected::Failed => return None,
        }
        let total: f64 = self.counter.instances()?.iter().map(|(_, v)| v).sum();
        Some(total.clamp(0.0, 100.0) as f32)
    }
}
