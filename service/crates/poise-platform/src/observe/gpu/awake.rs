//! 独立显卡（NVIDIA）最近有没有在干活：只读显卡内核里各引擎的累计运行时间（任务管理器的同一来源），
//! 不碰显卡本身。NVML 的查询会把 Optimus 笔记本上休眠中的独显叫醒（之后要过一阵才再休眠，白白耗电发热），
//! 读 NVML 之前先用它看一眼独显是不是本来就醒着。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::LUID;

use super::kmt;

/// NVIDIA 的 PCI 厂商编号。
const NVIDIA: u32 = 0x10DE;

/// 没找到显卡、读不到统计时，隔这么久才重新枚举（开机时驱动可能还没加载好，重装驱动后显卡编号会变）。
const REENUMERATE_EVERY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct DiscreteActivity {
    /// NVIDIA 显卡及其引擎数量，与枚举的时刻。
    adapters: Option<(Instant, Vec<(LUID, u32)>)>,
    /// 上次读到的各显卡累计运行时间。
    last: Vec<(LUID, u64)>,
}

impl DiscreteActivity {
    /// 上次问到现在，NVIDIA 显卡有没有运行过。第一次问到一块显卡时还没法比较，按没有算；
    /// 没有 NVIDIA 显卡、读不到统计时为 None（调用方自己决定）。每轮都要问（比较的是上一次的读数）。
    pub fn busy(&mut self) -> Option<bool> {
        let adapters = self.adapters();
        let mut busy = false;
        let mut read = Vec::with_capacity(adapters.len());
        let mut failed = false;
        for (luid, nodes) in adapters {
            // 一块显卡读不到（拔掉了外接显卡、驱动在重装）不影响别的。
            let Some(total) = (0..nodes)
                .map(|n| kmt::node_running(luid, n))
                .sum::<Option<u64>>()
            else {
                failed = true;
                continue;
            };
            let last = self.last.iter().find(|(l, _)| *l == luid).map(|(_, t)| *t);
            busy |= last.is_some_and(|last| total > last);
            read.push((luid, total));
        }
        if failed || read.is_empty() {
            self.stale();
        }
        if read.is_empty() {
            self.last.clear();
            return None;
        }
        self.last = read;
        Some(busy)
    }

    /// 要读的显卡（引擎数为 0 的不算）；需要时重新枚举。
    fn adapters(&mut self) -> Vec<(LUID, u32)> {
        let due = self
            .adapters
            .as_ref()
            .is_none_or(|(at, list)| list.is_empty() && at.elapsed() >= REENUMERATE_EVERY);
        if due {
            let list = kmt::adapters_of(NVIDIA)
                .into_iter()
                .filter(|&(_, nodes)| nodes > 0)
                .collect();
            self.adapters = Some((Instant::now(), list));
        }
        self.adapters
            .as_ref()
            .map_or_else(Vec::new, |(_, l)| l.clone())
    }

    /// 读不到了：离上次枚举够久时，下次重新枚举（列表空着、枚举时刻已够久，见 [`Self::adapters`]）。
    fn stale(&mut self) {
        if let Some((at, list)) = &mut self.adapters
            && at.elapsed() >= REENUMERATE_EVERY
        {
            list.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_look_counts_as_idle() {
        let mut d = DiscreteActivity::default();
        // 没有 NVIDIA 显卡的电脑为 None；有的话第一次没法比较，按没在干活算。
        assert_ne!(d.busy(), Some(true));
        let second = d.busy();
        println!("NVIDIA 显卡在干活：{second:?}");
    }
}
