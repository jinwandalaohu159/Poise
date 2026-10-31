//! 一行日志的格式：`时:分:秒 [级别] 内容`，按本地时间。

use std::fmt::Write;

use log::Level;
use windows::Win32::System::SystemInformation::GetLocalTime;

pub(super) fn format(level: Level, args: &std::fmt::Arguments) -> String {
    // SAFETY: 无参数，返回本地时间。
    let t = unsafe { GetLocalTime() };
    let mut line = String::with_capacity(96);
    let _ = writeln!(
        line,
        "{:02}:{:02}:{:02} [{level}] {args}",
        t.wHour, t.wMinute, t.wSecond
    );
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_has_the_time_the_level_and_ends_with_a_newline() {
        let line = format(Level::Warn, &format_args!("读取失败：{}", 5));
        assert_eq!(line.len(), "00:00:00 [WARN] 读取失败：5\n".len());
        assert_eq!(&line[2..3], ":");
        assert!(line.ends_with(" [WARN] 读取失败：5\n"), "{line}");
    }
}
