//! 读取可执行文件的版本信息（文件描述、产品名），用作界面上的友好名称。

use std::ffi::c_void;

use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};

use crate::win::wide::{WideString, from_wide};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionStrings {
    pub file_description: Option<String>,
    pub product_name: Option<String>,
    pub company_name: Option<String>,
    /// 原始文件名（保留大小写，例如 "Widgets.exe"）。
    pub original_filename: Option<String>,
}

pub fn read(path: &str) -> VersionStrings {
    let wpath = WideString::new(path);
    // SAFETY: 路径以 NUL 结尾；缓冲区大小来自系统。
    let size = unsafe { GetFileVersionInfoSizeW(wpath.as_pcwstr(), None) };
    if size == 0 {
        return VersionStrings::default();
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: 同上。
    if unsafe {
        GetFileVersionInfoW(
            wpath.as_pcwstr(),
            None,
            size,
            data.as_mut_ptr() as *mut c_void,
        )
    }
    .is_err()
    {
        return VersionStrings::default();
    }
    let langs = translations(&data);
    let query = |field: &str| {
        langs
            .iter()
            .find_map(|(lang, cp)| {
                query_string(
                    &data,
                    &format!(r"\StringFileInfo\{lang:04x}{cp:04x}\{field}"),
                )
            })
            .filter(|s| !s.trim().is_empty())
    };
    VersionStrings {
        file_description: query("FileDescription"),
        product_name: query("ProductName"),
        company_name: query("CompanyName"),
        original_filename: query("OriginalFilename"),
    }
}

fn translations(data: &[u8]) -> Vec<(u16, u16)> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let sub = WideString::new(r"\VarFileInfo\Translation");
    // SAFETY: data 是完整的版本信息块；返回的指针指向 data 内部。
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr() as *const c_void,
            sub.as_pcwstr(),
            &mut ptr,
            &mut len,
        )
    };
    let mut out = Vec::new();
    if ok.as_bool() && !ptr.is_null() {
        let count = len as usize / 4;
        // SAFETY: 系统保证 len 字节有效，每项为两个 u16。
        let pairs = unsafe { std::slice::from_raw_parts(ptr as *const u16, count * 2) };
        out.extend(pairs.chunks(2).map(|p| (p[0], p[1])));
    }
    // 常见的回退：英文和简体中文，Unicode 代码页。
    out.extend([(0x0409, 0x04b0), (0x0804, 0x04b0), (0x0409, 0x04e4)]);
    out
}

fn query_string(data: &[u8], sub_block: &str) -> Option<String> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let sub = WideString::new(sub_block);
    // SAFETY: 同 translations。
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr() as *const c_void,
            sub.as_pcwstr(),
            &mut ptr,
            &mut len,
        )
    };
    if !ok.as_bool() || ptr.is_null() || len == 0 {
        return None;
    }
    // SAFETY: len 是字符数（含结尾 NUL）。
    let s = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize) };
    Some(from_wide(s))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_notepad_description() {
        let v = super::read(r"C:\Windows\System32\notepad.exe");
        assert!(v.company_name.is_some_and(|c| c.contains("Microsoft")));
    }
}
