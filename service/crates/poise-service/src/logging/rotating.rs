//! 按大小轮换的日志文件：写满 [`MAX_BYTES`] 后把它改名为 `.1`（覆盖更早的那份）、重新开一个继续写。
//! 只在一行写完之后才换，不会把一行拆到两个文件里。服务常年运行，启动时才检查大小是不够的。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

/// 每个文件最大的字节数。连同上一份，最多占两倍。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub(super) struct RotatingFile {
    path: PathBuf,
    file: File,
    written: u64,
    /// 上一次写入以换行结束（可以在这里换文件）。
    at_line_start: bool,
}

impl RotatingFile {
    pub fn open(path: PathBuf) -> Option<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let written = file.metadata().map_or(0, |m| m.len());
        let mut f = Self {
            path,
            file,
            written,
            at_line_start: true,
        };
        if f.written >= MAX_BYTES {
            f.rotate();
        }
        Some(f)
    }

    /// 换文件；失败时继续写原来的文件（日志不能因为改名失败就丢）。
    fn rotate(&mut self) {
        let old = self.path.with_extension("1.log");
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(&self.path, &old).is_err() {
            return;
        }
        if let Ok(file) = File::create(&self.path) {
            self.file = file;
            self.written = 0;
        }
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.at_line_start && self.written >= MAX_BYTES {
            self.rotate();
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        if n > 0 {
            self.at_line_start = buf[n - 1] == b'\n';
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_files_move_aside_only_between_lines() {
        let dir = std::env::temp_dir().join(format!("poise-rotating-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");
        let mut f = RotatingFile::open(path.clone()).unwrap();
        f.written = MAX_BYTES;
        f.at_line_start = false;
        // 一行还没写完：继续写在原来的文件里。
        f.write_all(b"tail\n").unwrap();
        assert!(!dir.join("test.1.log").exists());
        f.write_all(b"next\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("test.1.log")).unwrap(),
            "tail\n"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
