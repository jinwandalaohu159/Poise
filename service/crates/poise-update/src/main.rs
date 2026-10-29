//! Poise 更新助手：界面里点"更新"后启动（复制到用户目录运行，安装时不占用安装目录里的文件）。
//! 显示一个进度窗口：先下载新版本，再安装并启动，完成后自动关闭。没有任何控制台窗口。
//!
//! 用法：poise-updater --dir <安装目录> [--lang <界面语言>]
#![windows_subsystem = "windows"]

mod updater;

fn main() {
    updater::run();
}
