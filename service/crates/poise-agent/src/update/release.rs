//! GitHub Releases 的"最新版本"：从接口返回的 JSON 里取出可以安装的版本号，并与当前版本比较。纯函数。

use serde::Deserialize;

#[derive(Deserialize)]
struct Release {
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
}

/// 安装程序在发布里的文件名。
pub fn installer_name(version: &str) -> String {
    format!("Poise-Setup-{version}.exe")
}

/// 最新发布的版本号（不带 "v"）。草稿、预发布、标签不是 vX.Y.Z、或者没有附带安装程序时返回 None。
pub fn installable_version(json: &str) -> Option<String> {
    let release: Release = serde_json::from_str(json).ok()?;
    if release.draft || release.prerelease {
        return None;
    }
    let version = release.tag_name.strip_prefix('v')?;
    parse(version)?;
    let installer = installer_name(version);
    release
        .assets
        .iter()
        .any(|a| a.name == installer)
        .then(|| version.to_string())
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

    fn release(tag: &str, asset: &str, draft: bool, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{prerelease},
                "assets":[{{"name":"{asset}","size":1}}],"body":"notes"}}"#
        )
    }

    #[test]
    fn a_published_release_with_its_installer_is_installable() {
        let json = release("v0.3.0", "Poise-Setup-0.3.0.exe", false, false);
        assert_eq!(installable_version(&json).as_deref(), Some("0.3.0"));
    }

    #[test]
    fn drafts_prereleases_odd_tags_and_missing_installers_are_ignored() {
        let cases = [
            release("v0.3.0", "Poise-Setup-0.3.0.exe", true, false),
            release("v0.3.0", "Poise-Setup-0.3.0.exe", false, true),
            release("0.3.0", "Poise-Setup-0.3.0.exe", false, false),
            release("v0.3", "Poise-Setup-0.3.exe", false, false),
            release("v0.3.0", "Poise-Setup-0.2.0.exe", false, false),
            "not json".to_string(),
        ];
        for json in cases {
            assert_eq!(installable_version(&json), None, "{json}");
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
