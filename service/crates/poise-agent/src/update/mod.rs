//! 检查更新：后台线程定期查询 GitHub 上的最新发布，有新版本时通知托盘（托盘弹出提示、菜单里出现"更新"）。
//! 只读取版本号；用户在界面里确认后，由更新助手（poise-updater）下载、安装并启动新版本。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// 启动后多久第一次检查（避开登录时最忙的一段）。
const FIRST_CHECK: Duration = Duration::from_secs(2 * 60);
/// 之后每隔多久检查一次。
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);
/// 检查失败（没联网、刚醒来网络还没好）后，依次隔这么久再试，之后按 [`CHECK_EVERY`]。
const RETRY_AFTER: [Duration; 3] = [
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
];

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
    let mut failures = 0usize;
    std::thread::sleep(FIRST_CHECK);
    loop {
        match poise_update::latest_release() {
            Ok(latest) => {
                failures = 0;
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
            Err(e) => {
                if failures == 0 {
                    log::info!("检查更新失败（稍后重试）：{e}");
                }
                failures += 1;
            }
        }
        let wait = match failures {
            0 => CHECK_EVERY,
            n => RETRY_AFTER.get(n - 1).copied().unwrap_or(CHECK_EVERY),
        };
        sleep_wall_clock(wait);
    }
}

/// 按钟表时间等：电脑睡眠的时间也算进去（`thread::sleep` 不算睡眠的时间，"每 4 小时"会变成
/// "醒着 4 小时"，经常合上盖子的笔记本可能好几天都不检查）。分段睡，每段最多 [`SLEEP_STEP`]。
/// 醒着的时间到了也算等够：系统时间被往回调了（手动改、对时）时不会一直等下去。
fn sleep_wall_clock(wait: Duration) {
    const SLEEP_STEP: Duration = Duration::from_secs(10 * 60);
    let until = SystemTime::now() + wait;
    let awake_until = Instant::now() + wait;
    while let Ok(left) = until.duration_since(SystemTime::now()) {
        let awake_left = awake_until.saturating_duration_since(Instant::now());
        if left.is_zero() || awake_left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(awake_left).min(SLEEP_STEP));
    }
}
