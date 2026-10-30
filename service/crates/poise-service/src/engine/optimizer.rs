//! 系统优化项：列出状态、应用、还原。项目本身来自目录（[`super::catalog`]）；电源项被别处改回时由 [`super::power_guard`] 补回。

use std::sync::Arc;

use poise_core::optimize::{self, ApplyReport, ItemStatus, OptimizationItem};
use poise_platform::settings::user::UserContext;

use poise_protocol::{ErrorCode, Event};

use super::Engine;
use super::catalog::Catalog;
use super::changes;
use super::error::RequestError;

impl Engine {
    /// 优化项目录（请求线程通常已经刷新过，见 [`super::catalog`]）。
    fn optimization_items(&self) -> Arc<Vec<OptimizationItem>> {
        self.catalog.items(self.changes.store.user().as_ref())
    }

    /// 刷新目录所需的：目录本身与当前用户。请求线程短暂持有引擎锁取得它们，之后在锁外枚举。
    pub fn catalog_handle(&self) -> (Arc<Catalog>, Option<UserContext>) {
        (self.catalog.clone(), self.changes.store.user())
    }

    pub fn list_optimizations(&self) -> Vec<ItemStatus> {
        optimize::status(
            &self.optimization_items(),
            &self.changes.store,
            self.changes.journal(),
        )
    }

    fn optimization_item(&self, id: &str) -> Result<OptimizationItem, RequestError> {
        self.optimization_items()
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or_else(|| RequestError::new(ErrorCode::NotFound, "没有这个优化项"))
    }

    pub fn apply_optimization(&mut self, id: &str) -> Result<(), RequestError> {
        let item = self.optimization_item(id)?;
        let now = self.now();
        self.changes
            .edit(|s, j| optimize::apply(&item, s, j, now))?;
        self.note_applied(vec![id.to_string()]);
        Ok(())
    }

    /// 还原一项：Poise 改过的回到修改前，别处改的（用户自己或其他工具）回到系统默认。
    pub fn revert_optimization(&mut self, id: &str) -> Result<(), RequestError> {
        if !changes::is_optimization(id) {
            return Err(RequestError::new(
                ErrorCode::NotFound,
                "这项修改不属于系统优化",
            ));
        }
        if self.changes.journal().owns(id) {
            self.changes.revert(id)?;
        } else {
            let item = self.optimization_item(id)?;
            self.changes.restore_default(&item)?;
        }
        self.note_reverted(vec![id.to_string()]);
        Ok(())
    }

    pub fn apply_recommended(&mut self) -> Result<ApplyReport, RequestError> {
        let items = self.optimization_items();
        let now = self.now();
        let report = self
            .changes
            .edit(|s, j| Ok(optimize::apply_recommended(&items, s, j, now)))?;
        self.note_applied(report.applied.clone());
        Ok(report)
    }

    pub fn revert_optimizations(&mut self) -> Result<Vec<String>, RequestError> {
        let reverted = self.changes.revert_optimizations()?;
        self.note_reverted(reverted.clone());
        Ok(reverted)
    }

    fn note_applied(&mut self, ids: Vec<String>) {
        if !ids.is_empty() {
            self.logbook.event(Event::OptimizationsApplied { ids });
        }
    }

    fn note_reverted(&mut self, ids: Vec<String>) {
        if !ids.is_empty() {
            self.logbook.event(Event::OptimizationsReverted { ids });
        }
    }
}
