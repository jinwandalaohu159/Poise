//! 系统优化项（静态 + 动态枚举）。

use poise_core::optimize::{self, ApplyReport, ItemStatus, OptimizationItem};
use poise_platform::settings::dynamic_items;
use poise_platform::settings::user::UserContext;
use poise_platform::win::registry;

use poise_protocol::ErrorCode;

use super::Engine;
use super::changes;
use super::error::RequestError;

impl Engine {
    fn optimization_items(&self) -> Vec<OptimizationItem> {
        let user = self.changes.store.user();
        let mut items = optimize::static_items(wsl_installed(user.as_ref()));
        items.extend(dynamic_items::context_menu_items(user.as_ref()));
        items.extend(dynamic_items::navigation_items(user.as_ref()));
        items.extend(dynamic_items::startup_items(user.as_ref()));
        items
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
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| RequestError::new(ErrorCode::NotFound, "没有这个优化项"))
    }

    pub fn apply_optimization(&mut self, id: &str) -> Result<(), RequestError> {
        let item = self.optimization_item(id)?;
        let now = self.now();
        Ok(self
            .changes
            .apply(&item.id, &item.target, &item.optimized, now)?)
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
            return Ok(self.changes.revert(id)?);
        }
        let item = self.optimization_item(id)?;
        Ok(self.changes.restore_default(&item)?)
    }

    pub fn apply_recommended(&mut self) -> Result<ApplyReport, RequestError> {
        let items = self.optimization_items();
        let now = self.now();
        Ok(self
            .changes
            .edit(|s, j| Ok(optimize::apply_recommended(&items, s, j, now)))?)
    }

    pub fn revert_optimizations(&mut self) -> Result<Vec<String>, RequestError> {
        Ok(self.changes.revert_optimizations()?)
    }
}

fn wsl_installed(user: Option<&UserContext>) -> bool {
    let Some(user) = user else { return false };
    let (root, prefix) = user.registry_root();
    registry::open(
        root,
        &format!(r"{prefix}Software\Microsoft\Windows\CurrentVersion\Lxss"),
        false,
    )
    .is_ok()
}
