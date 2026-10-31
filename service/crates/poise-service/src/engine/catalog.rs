//! 系统优化项的目录：静态项，加上这台电脑上实际注册的动态项（右键菜单插件、导航栏入口、开机启动项）。
//!
//! 枚举动态项要读注册表和每个插件的版本信息，可能要几百毫秒。所以由请求线程在**不持有引擎锁**时枚举
//! （见 `ipc_server::route`），引擎只读取最近的结果——界面打开优化页时，调度不会被卡住。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poise_core::optimize::{self, OptimizationItem};
use poise_platform::settings::dynamic_items;
use poise_platform::settings::user::UserContext;
use poise_platform::win::registry;

#[derive(Default)]
pub struct Catalog {
    cache: Mutex<Option<Cached>>,
}

struct Cached {
    at: Instant,
    user: Option<UserContext>,
    items: Arc<Vec<OptimizationItem>>,
}

impl Catalog {
    /// 目录比 `max_age` 旧（或换了用户）时重新枚举。不持有任何锁时调用。
    pub fn refresh(&self, user: Option<&UserContext>, max_age: Duration) {
        if self.cached(user, max_age).is_some() {
            return;
        }
        // 枚举时不占着缓存的锁：引擎读取目录不必等它。
        let items = Arc::new(enumerate(user));
        self.store(user, items);
    }

    /// 最近的目录；没有（或用户不同）时现在枚举。
    pub fn items(&self, user: Option<&UserContext>) -> Arc<Vec<OptimizationItem>> {
        if let Some(items) = self.cached(user, Duration::MAX) {
            return items;
        }
        let items = Arc::new(enumerate(user));
        self.store(user, items.clone());
        items
    }

    fn cached(
        &self,
        user: Option<&UserContext>,
        max_age: Duration,
    ) -> Option<Arc<Vec<OptimizationItem>>> {
        let cache = self.cache.lock().ok()?;
        cache
            .as_ref()
            .filter(|c| c.user.as_ref() == user && c.at.elapsed() <= max_age)
            .map(|c| c.items.clone())
    }

    fn store(&self, user: Option<&UserContext>, items: Arc<Vec<OptimizationItem>>) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(Cached {
                at: Instant::now(),
                user: user.cloned(),
                items,
            });
        }
    }
}

fn enumerate(user: Option<&UserContext>) -> Vec<OptimizationItem> {
    let mut items = optimize::static_items(wsl_installed(user));
    items.extend(dynamic_items::context_menu_items(user));
    items.extend(dynamic_items::navigation_items(user));
    items.extend(dynamic_items::startup_items(user));
    items
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recent_catalog_is_reused() {
        let catalog = Catalog::default();
        let user = UserContext::current_process_user();
        catalog.refresh(user.as_ref(), Duration::ZERO);
        let first = catalog.items(user.as_ref());
        // 刚枚举过：直接用，不重新枚举。
        assert!(Arc::ptr_eq(&first, &catalog.items(user.as_ref())));
        catalog.refresh(user.as_ref(), Duration::from_secs(60));
        assert!(Arc::ptr_eq(&first, &catalog.items(user.as_ref())));
    }
}
