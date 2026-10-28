//! 检查更新：后台线程定期查询 GitHub 上的最新发布，有新版本时通知托盘（托盘弹出提示、菜单里出现"更新"）。
//! 只连接 GitHub，只读取版本号；下载和安装由界面负责（用户确认后）。

mod http;
mod release;

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 启动后多久第一次检查（避开登录时最忙的一段）。
const FIRST_CHECK: Duration = Duration::from_secs(2 * 60);
/// 之后每隔多久检查一次。
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

const API_HOST: &str = "api.github.com";
const LATEST_RELEASE: &str = "/repos/jinwandalaohu159/Poise/releases/latest";

/// 本程序的版本（与界面、服务同一个版本号）。
const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub struct UpdateChecker {
    available: Arc<Mutex<Option<String>>>,
}

impl UpdateChecker {
    /// 启动后台线程。发现新版本时调用 `notify`（在后台线程上调用，应只投递消息）。
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let available = Arc::new(Mutex::new(None));
        let shared = available.clone();
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || run(&shared, &notify));
        if let Err(e) = spawned {
            log::warn!("无法启动更新检查：{e}");
        }
        Self { available }
    }

    /// 已发现、比当前新的版本号。
    pub fn available(&self) -> Option<String> {
        self.available.lock().ok().and_then(|v| v.clone())
    }
}

fn run(available: &Mutex<Option<String>>, notify: &dyn Fn()) {
    let mut failing = false;
    std::thread::sleep(FIRST_CHECK);
    loop {
        match latest_version() {
            Ok(latest) => {
                failing = false;
                let newer = latest.filter(|v| release::is_newer(v, CURRENT));
                let changed = available
                    .lock()
                    .map(|mut a| std::mem::replace(&mut *a, newer.clone()) != newer)
                    .unwrap_or(false);
                if changed && let Some(v) = &newer {
                    log::info!("发现新版本 {v}（当前 {CURRENT}）");
                    notify();
                }
            }
            // 连续失败只记一次，恢复后重新计。
            Err(e) if !failing => {
                failing = true;
                log::info!("检查更新失败（稍后重试）：{e}");
            }
            Err(_) => {}
        }
        std::thread::sleep(CHECK_EVERY);
    }
}

/// 最新可安装的版本号；最新发布没有安装程序时为 None。
fn latest_version() -> Result<Option<String>, String> {
    let (status, body) = http::get(
        API_HOST,
        LATEST_RELEASE,
        &format!("Poise/{CURRENT}"),
        "Accept: application/vnd.github+json\r\n",
    )?;
    match status {
        200 => Ok(release::installable_version(&String::from_utf8_lossy(
            &body,
        ))),
        // 还没有任何发布。
        404 => Ok(None),
        s => Err(format!("GitHub 返回 {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 访问真实的 GitHub 接口，手动运行：cargo test -p poise-agent live_latest -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_latest_release_is_readable() {
        let latest = latest_version().expect("请求失败");
        println!("最新可安装版本：{latest:?}");
        assert!(latest.is_some());
    }
}
