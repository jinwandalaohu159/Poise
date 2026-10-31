//! 界面轮询的只读请求（状态每秒一次，程序列表每两秒，清理概览每三秒……）在两轮采样之间结果不变：
//! 每种请求留一份写好的回复，再问时直接给，不在引擎锁里重新整理、序列化（程序列表要整理分组、
//! 给几百个进程排序，清理概览要重新做一遍清理计划）。
//!
//! 引擎的状态每变一次就全部作废（[`Answers::invalidate`]）：一轮采样、立即重新调度、修改请求、
//! 一键清理、界面断开。

use poise_protocol::{Request, ResponseEnvelope};

/// 可以留着回复的只读请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Query {
    Status,
    Apps,
    CleanupOverview,
    Diagnose,
    Trend,
    Cores,
}

impl Query {
    pub fn of(request: &Request) -> Option<Self> {
        Some(match request {
            Request::GetStatus => Self::Status,
            Request::ListApps => Self::Apps,
            Request::CleanupOverview => Self::CleanupOverview,
            Request::Diagnose => Self::Diagnose,
            Request::Trend => Self::Trend,
            Request::Cores => Self::Cores,
            _ => return None,
        })
    }
}

/// 留着的回复（编号换成这次请求的）。
#[derive(Default)]
pub(super) struct Answers {
    cached: Vec<(Query, ResponseEnvelope)>,
}

impl Answers {
    pub fn get(&self, query: Query, id: u64) -> Option<ResponseEnvelope> {
        self.cached
            .iter()
            .find(|(q, _)| *q == query)
            .map(|(_, r)| ResponseEnvelope { id, ..r.clone() })
    }

    pub fn put(&mut self, query: Query, response: &ResponseEnvelope) {
        self.cached.retain(|(q, _)| *q != query);
        self.cached.push((query, response.clone()));
    }

    pub fn invalidate(&mut self) {
        self.cached.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_reused_with_the_new_id_until_invalidated() {
        let mut a = Answers::default();
        assert!(a.get(Query::Status, 1).is_none());
        a.put(Query::Status, &ResponseEnvelope::ok(1, &5));
        let again = a.get(Query::Status, 2).unwrap();
        assert_eq!(again.id, 2);
        assert_eq!(again.result.unwrap().get(), "5");
        assert!(a.get(Query::Apps, 3).is_none());
        a.invalidate();
        assert!(a.get(Query::Status, 4).is_none());
        assert_eq!(Query::of(&Request::GetStatus), Some(Query::Status));
        assert_eq!(Query::of(&Request::ApplyRecommended), None);
    }
}
