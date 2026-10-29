//! 命令行参数。

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Args {
    /// 已安装的 Poise 所在目录：读取当前版本，安装后从这里启动托盘代理。
    pub dir: PathBuf,
    /// 界面语言（例如 "zh-CN"）；没有时跟随系统。
    pub lang: Option<String>,
    /// %LOCALAPPDATA%\Poise：日志，以及下载的安装程序（updates 子目录）。
    pub data_dir: PathBuf,
}

impl Args {
    pub fn parse(args: impl Iterator<Item = String>) -> Option<Self> {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Self::parse_with(args, PathBuf::from(local).join("Poise"))
    }

    fn parse_with(mut args: impl Iterator<Item = String>, data_dir: PathBuf) -> Option<Self> {
        let mut dir = None;
        let mut lang = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dir" => dir = Some(PathBuf::from(args.next()?)),
                "--lang" => lang = Some(args.next()?),
                _ => return None,
            }
        }
        Some(Self {
            dir: dir?,
            lang,
            data_dir,
        })
    }

    pub fn updates_dir(&self) -> PathBuf {
        self.data_dir.join("updates")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Args> {
        Args::parse_with(args.iter().map(|s| s.to_string()), PathBuf::from("data"))
    }

    #[test]
    fn the_install_directory_is_required() {
        let a = parse(&["--dir", r"D:\Apps\Poise", "--lang", "en-US"]).unwrap();
        assert_eq!(a.dir, PathBuf::from(r"D:\Apps\Poise"));
        assert_eq!(a.lang.as_deref(), Some("en-US"));
        assert!(parse(&["--lang", "zh-CN"]).is_none());
        assert!(parse(&["--dir"]).is_none());
        assert!(parse(&["--dir", "x", "--unknown"]).is_none());
    }
}
