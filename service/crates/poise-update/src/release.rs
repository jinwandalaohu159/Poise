//! GitHub Releases 的"最新版本"：从接口返回的 JSON 里取出可以安装的发布，并比较版本号。纯函数。
//!
//! 与界面（`GitHubReleases.cs`）规则一致：草稿、预发布、标签不是 vX.Y.Z、没有与版本号一致的安装程序、
//! 安装程序没有 SHA-256 校验值的发布都不算——宁可不更新，也不安装无法校验的文件。

use serde::Deserialize;

/// 一个可以安装的发布。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// 版本号（不带 "v"）。
    pub version: String,
    pub installer_url: String,
    /// 安装程序的大小（字节）。
    pub installer_size: u64,
    /// 安装程序的 SHA-256（小写十六进制）。
    pub sha256: String,
}

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// 安装程序在发布里的文件名。
pub fn installer_name(version: &str) -> String {
    format!("Poise-Setup-{version}.exe")
}

/// 解析"最新版本"接口的返回；不可安装时返回 None。
pub fn parse_latest(json: &str) -> Option<Release> {
    let latest: Latest = serde_json::from_str(json).ok()?;
    if latest.draft || latest.prerelease {
        return None;
    }
    let version = latest.tag_name.strip_prefix('v')?;
    parse(version)?;
    let name = installer_name(version);
    let asset = latest.assets.into_iter().find(|a| a.name == name)?;
    if !asset.browser_download_url.starts_with("https://") {
        return None;
    }
    Some(Release {
        version: version.to_string(),
        installer_url: asset.browser_download_url,
        installer_size: asset.size,
        sha256: sha256(asset.digest.as_deref()?)?,
    })
}

/// "sha256:abc…" → "abc…"；其他算法或格式不对时返回 None。
fn sha256(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?.to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// `candidate` 是否比 `current` 新（按主版本、次版本、修订号逐级比较）。解析不了时不算新。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

/// "1.2.3" → (1, 2, 3)。只接受三段数字。
fn parse(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    fn release(tag: &str, asset: &str, draft: bool, prerelease: bool, digest: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{prerelease},
                "assets":[{{"name":"{asset}","size":42,"digest":"{digest}",
                "browser_download_url":"https://github.com/x/releases/download/{tag}/{asset}"}}],
                "body":"notes"}}"#
        )
    }

    #[test]
    fn a_published_release_with_a_verifiable_installer_is_installable() {
        let json = release("v0.3.0", "Poise-Setup-0.3.0.exe", false, false, DIGEST);
        assert_eq!(
            parse_latest(&json),
            Some(Release {
                version: "0.3.0".into(),
                installer_url:
                    "https://github.com/x/releases/download/v0.3.0/Poise-Setup-0.3.0.exe".into(),
                installer_size: 42,
                sha256: DIGEST["sha256:".len()..].to_ascii_lowercase(),
            })
        );
    }

    #[test]
    fn drafts_prereleases_odd_tags_missing_installers_and_digests_are_ignored() {
        let cases = [
            release("v0.3.0", "Poise-Setup-0.3.0.exe", true, false, DIGEST),
            release("v0.3.0", "Poise-Setup-0.3.0.exe", false, true, DIGEST),
            release("0.3.0", "Poise-Setup-0.3.0.exe", false, false, DIGEST),
            release("v0.3", "Poise-Setup-0.3.exe", false, false, DIGEST),
            release("v0.3.0", "Poise-Setup-0.2.0.exe", false, false, DIGEST),
            release("v0.3.0", "Poise-Setup-0.3.0.exe", false, false, "sha1:abc"),
            release(
                "v0.3.0",
                "Poise-Setup-0.3.0.exe",
                false,
                false,
                "sha256:xyz",
            ),
            r#"{"tag_name":"v0.3.0","assets":[{"name":"Poise-Setup-0.3.0.exe","size":1,
               "browser_download_url":"https://x/y"}]}"#
                .to_string(),
            "not json".to_string(),
        ];
        for json in cases {
            assert_eq!(parse_latest(&json), None, "{json}");
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("0.3.0-beta", "0.2.0"));
        assert!(!is_newer("0.3.0", "dev"));
    }
}
