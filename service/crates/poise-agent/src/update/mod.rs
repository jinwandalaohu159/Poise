//! 检查更新：后台线程定期查询 GitHub 上的最新发布，有新版本时通知托盘（托盘弹出提示、菜单里出现"更新"）。
//! 只读取版本号；用户在界面里确认后，由更新助手（poise-updater）下载、安装并启动新版本。

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 启动后多久第一次检查（避开登录时最忙的一段）。
const FIRST_CHECK: Duration = Duration::from_secs(2 * 60);
/// 之后每隔多久检查一次。
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

use poise_update::CURRENT;

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
        match poise_update::latest_release() {
            Ok(latest) => {
                failing = false;
                let newer = latest
                    .map(|r| r.version)
                    .filter(|v| poise_update::is_newer(v, CURRENT));
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
