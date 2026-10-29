//! Poise 的更新。
//!
//! - 库：查询 GitHub 上最新的可安装发布（托盘代理定期检查、更新助手开始更新时使用）。
//! - 程序 `poise-updater`（见 `main.rs`）：界面里点"更新"后启动，下载、安装并启动新版本，全程显示进度。
//!
//! 只连接 GitHub。

pub mod http;
pub mod release;

pub use release::{Release, is_newer};

const LATEST_RELEASE: &str = "https://api.github.com/repos/jinwandalaohu159/Poise/releases/latest";

/// 版本信息只有几 KB，超过说明不是预期的内容。
const MAX_RELEASE_JSON: usize = 1 << 20;

/// 本程序的版本（与界面、服务同一个版本号）。
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// 请求时带的 User-Agent（GitHub 要求）。
pub fn user_agent() -> String {
    format!("Poise/{CURRENT}")
}

/// 最新的可安装发布；还没有任何发布、或者最新发布不可安装（见 [`release::parse_latest`]）时为 None。
pub fn latest_release() -> Result<Option<Release>, String> {
    let response = http::get(
        LATEST_RELEASE,
        &user_agent(),
        "Accept: application/vnd.github+json\r\n",
    )?;
    match response.status() {
        200 => {
            let body = response.read_to_end(MAX_RELEASE_JSON)?;
            Ok(release::parse_latest(&String::from_utf8_lossy(&body)))
        }
        // 还没有任何发布。
        404 => Ok(None),
        s => Err(format!("GitHub 返回 {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 访问真实的 GitHub 接口，手动运行：cargo test -p poise-update live_latest -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_latest_release_is_readable() {
        let latest = latest_release().expect("请求失败");
        println!("最新可安装发布：{latest:?}");
        assert!(latest.is_some());
    }
}
