//! 环境变量展开（注册表里 REG_EXPAND_SZ 的路径，例如 `%SystemDrive%\Users\x`）。

use windows::Win32::System::Environment::ExpandEnvironmentStringsW;

use super::wide::WideString;

/// 按本进程的环境展开 `%变量%`；展开失败时原样返回。
pub fn expand(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let source = WideString::new(text);
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: 输入以 NUL 结尾；输出缓冲区长度与传入一致。
        let needed =
            unsafe { ExpandEnvironmentStringsW(source.as_pcwstr(), Some(&mut buf)) } as usize;
        if needed == 0 {
            return text.to_string();
        }
        if needed <= buf.len() {
            // 返回的长度含结尾的 NUL。
            return String::from_utf16_lossy(&buf[..needed - 1]);
        }
        buf.resize(needed, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_are_expanded_and_plain_text_is_kept() {
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand(r"%SystemRoot%\x"), format!(r"{root}\x"));
        assert_eq!(expand(r"C:\Users\a"), r"C:\Users\a");
        assert_eq!(expand("%NoSuchPoiseVariable%"), "%NoSuchPoiseVariable%");
    }
}
