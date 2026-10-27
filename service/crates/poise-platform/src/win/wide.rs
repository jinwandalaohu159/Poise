//! UTF-16 字符串转换。

use windows::core::PCWSTR;

/// 以 NUL 结尾的 UTF-16 缓冲区，生命周期内可作为 PCWSTR 传给系统 API。
pub struct WideString(Vec<u16>);

impl WideString {
    pub fn new(s: &str) -> Self {
        Self(s.encode_utf16().chain(std::iter::once(0)).collect())
    }

    pub fn as_pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }

    pub fn as_mut_ptr(&mut self) -> *mut u16 {
        self.0.as_mut_ptr()
    }
}

/// 把 UTF-16 缓冲区（到第一个 NUL 为止）转成 String。
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
