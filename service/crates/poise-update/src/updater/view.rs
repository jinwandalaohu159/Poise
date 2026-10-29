//! 进度窗口上显示什么（纯函数）：每个阶段的标题、说明和进度，以及结束时的页面。

use std::time::Duration;

use super::flow::INSTALLED_AT;
use super::progress::{Failure, Outcome, Stage};
use super::text::{Text, megabytes};

/// 某一时刻窗口上显示的内容。
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub instruction: String,
    pub content: String,
    /// 当前这一段的进度（0～1）。
    pub fraction: f32,
}

/// 结束时的页面。
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
    pub instruction: String,
    pub content: String,
    pub bar: Bar,
    pub auto_close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    None,
    Full,
    Error,
}

/// 进行中的显示内容（纯函数）。
pub fn view(text: &Text, version: Option<&str>, stage: &Stage) -> View {
    let v = version.unwrap_or("");
    let (instruction, content, fraction) = match stage {
        Stage::Checking => (
            text.get("updater.checking"),
            text.get("updater.checking.detail"),
            0.0,
        ),
        Stage::Downloading { done, total } => (
            text.format("updater.downloading", &[v]),
            text.format(
                "updater.downloading.detail",
                &[&megabytes(*done), &megabytes(*total)],
            ),
            if *total == 0 {
                0.0
            } else {
                *done as f32 / *total as f32
            },
        ),
        Stage::Installing { since } => (
            text.format("updater.installing", &[v]),
            text.get("updater.installing.detail"),
            installing_fraction(since.elapsed()),
        ),
        Stage::Starting { fraction } => (
            text.format("updater.starting", &[v]),
            text.get("updater.starting.detail"),
            *fraction,
        ),
        Stage::Finished(_) => (String::new(), String::new(), 1.0),
    };
    View {
        instruction,
        content,
        fraction: fraction.clamp(0.0, 1.0),
    }
}

/// 安装程序不报告进度：按已经过去的时间平滑地接近"安装完成"的位置，不会到达也不会倒退。
fn installing_fraction(elapsed: Duration) -> f32 {
    INSTALLED_AT * (1.0 - (-elapsed.as_secs_f32() / 15.0).exp())
}

/// 结束页面的内容（纯函数）。
pub fn ending(text: &Text, version: Option<&str>, outcome: &Outcome) -> Ending {
    let v = version.unwrap_or("");
    let failed = |reason: String| Ending {
        instruction: text.get("updater.failed"),
        content: reason,
        bar: Bar::Error,
        auto_close: false,
    };
    match outcome {
        Outcome::Updated => Ending {
            instruction: text.format("updater.done", &[v]),
            content: text.get("updater.done.detail"),
            bar: Bar::Full,
            auto_close: true,
        },
        Outcome::UpToDate => Ending {
            instruction: text.get("updater.up_to_date"),
            content: text.get("updater.up_to_date.detail"),
            bar: Bar::None,
            auto_close: false,
        },
        Outcome::Failed(f) => failed(match f {
            Failure::Network(_) | Failure::Cancelled => text.get("updater.failed.network"),
            Failure::Verification => text.get("updater.failed.verification"),
            Failure::Declined => text.get("updater.failed.declined"),
            Failure::Launch(_) => text.get("updater.failed.launch"),
            Failure::Install(code) => text.format("updater.failed.install", &[&code.to_string()]),
            Failure::Start => text.get("updater.failed.start"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> Text {
        Text::new(Some("zh-CN"))
    }

    #[test]
    fn download_shows_sizes_and_fraction() {
        let v = view(
            &text(),
            Some("0.5.0"),
            &Stage::Downloading {
                done: 25 * 1024 * 1024,
                total: 100 * 1024 * 1024,
            },
        );
        assert!(v.instruction.contains("0.5.0"));
        assert!(v.content.contains("25.0 MB") && v.content.contains("100.0 MB"));
        assert_eq!(v.fraction, 0.25);
    }

    #[test]
    fn installing_moves_smoothly_without_reaching_installed() {
        let at = |s: u64| installing_fraction(Duration::from_secs(s));
        assert_eq!(at(0), 0.0);
        assert!(at(5) < at(10) && at(10) < at(60));
        assert!(at(600) <= INSTALLED_AT);
    }

    #[test]
    fn endings_explain_what_happened() {
        let t = text();
        let done = ending(&t, Some("0.5.0"), &Outcome::Updated);
        assert!(done.instruction.contains("0.5.0") && done.auto_close);
        assert_eq!(done.bar, Bar::Full);
        let declined = ending(&t, Some("0.5.0"), &Outcome::Failed(Failure::Declined));
        assert_eq!(declined.bar, Bar::Error);
        assert!(!declined.auto_close);
        let failed = ending(&t, None, &Outcome::Failed(Failure::Install(4)));
        assert!(failed.content.contains('4'));
        for f in [
            Failure::Network(String::new()),
            Failure::Verification,
            Failure::Declined,
            Failure::Launch(String::new()),
            Failure::Install(1),
            Failure::Start,
        ] {
            let e = ending(&t, None, &Outcome::Failed(f));
            assert!(!e.content.starts_with("updater."), "{}", e.content);
        }
    }
}
