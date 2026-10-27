//! 注册表读写的薄封装。

use poise_core::optimize::Value;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE, REG_BINARY, REG_DWORD, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::win::wide::{WideString, from_wide};

pub struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: 句柄由本对象独占。
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

pub fn open(root: HKEY, path: &str, write: bool) -> Result<RegKey, WIN32_ERROR> {
    let access: REG_SAM_FLAGS = if write {
        KEY_READ | KEY_WRITE
    } else {
        KEY_READ
    };
    let w = WideString::new(path);
    let mut key = HKEY::default();
    // SAFETY: 输出到本地句柄。
    unsafe {
        RegOpenKeyExW(
            root,
            w.as_pcwstr(),
            None,
            access | KEY_WOW64_64KEY,
            &mut key,
        )
    }
    .ok()
    .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
    Ok(RegKey(key))
}

pub fn create(root: HKEY, path: &str) -> Result<RegKey, WIN32_ERROR> {
    let w = WideString::new(path);
    let mut key = HKEY::default();
    // SAFETY: 输出到本地句柄；不改变已存在键的安全描述符。
    unsafe {
        RegCreateKeyExW(
            root,
            w.as_pcwstr(),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            None,
            &mut key,
            None,
        )
    }
    .ok()
    .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
    Ok(RegKey(key))
}

/// 读取一个值；值不存在时返回 Ok(None)。
pub fn read_value(root: HKEY, path: &str, name: &str) -> Result<Option<Value>, WIN32_ERROR> {
    match open(root, path, false) {
        Ok(k) => k.read(name),
        Err(e) if e == ERROR_FILE_NOT_FOUND => Ok(None),
        Err(e) => Err(e),
    }
}

impl RegKey {
    pub fn read(&self, name: &str) -> Result<Option<Value>, WIN32_ERROR> {
        let w = WideString::new(name);
        let mut ty = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        // SAFETY: 先询问大小。
        let r = unsafe {
            RegQueryValueExW(
                self.0,
                w.as_pcwstr(),
                None,
                Some(&mut ty),
                None,
                Some(&mut len),
            )
        };
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        r.ok()
            .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
        let mut data = vec![0u8; len as usize];
        // SAFETY: 缓冲区大小为系统给出的长度。
        unsafe {
            RegQueryValueExW(
                self.0,
                w.as_pcwstr(),
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut len),
            )
        }
        .ok()
        .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())?;
        data.truncate(len as usize);
        Ok(Some(decode(ty, data)))
    }

    pub fn write(&self, name: &str, value: &Value) -> Result<(), WIN32_ERROR> {
        let w = WideString::new(name);
        let (ty, bytes) = encode(value);
        // SAFETY: 数据缓冲区在调用期间有效。
        unsafe { RegSetValueExW(self.0, w.as_pcwstr(), None, ty, Some(&bytes)) }
            .ok()
            .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
    }

    /// 删除一个值；值本来就不存在也算成功。
    pub fn delete(&self, name: &str) -> Result<(), WIN32_ERROR> {
        let w = WideString::new(name);
        // SAFETY: 名称以 NUL 结尾。
        let r = unsafe { RegDeleteValueW(self.0, w.as_pcwstr()) };
        if r == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        r.ok()
            .map_err(|e| WIN32_ERROR::from_error(&e).unwrap_or_default())
    }

    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for index in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            // SAFETY: 缓冲区长度与 len 一致。
            let r = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if r != windows::Win32::Foundation::ERROR_SUCCESS {
                break;
            }
            out.push(from_wide(&buf[..len as usize]));
        }
        out
    }

    pub fn values(&self) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 512];
            let mut name_len = name.len() as u32;
            let mut ty = 0u32;
            let mut data = vec![0u8; 8192];
            let mut data_len = data.len() as u32;
            // SAFETY: 缓冲区长度与传入长度一致。
            let r = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut ty),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if r == ERROR_NO_MORE_ITEMS {
                break;
            }
            if r != windows::Win32::Foundation::ERROR_SUCCESS {
                continue;
            }
            data.truncate(data_len as usize);
            out.push((
                from_wide(&name[..name_len as usize]),
                decode(REG_VALUE_TYPE(ty), data),
            ));
        }
        out
    }

    /// 键的默认值（字符串）。
    pub fn default_string(&self) -> Option<String> {
        match self.read("") {
            Ok(Some(Value::String(s))) if !s.is_empty() => Some(s),
            _ => None,
        }
    }
}

fn decode(ty: REG_VALUE_TYPE, data: Vec<u8>) -> Value {
    match ty {
        REG_DWORD if data.len() >= 4 => {
            Value::Dword(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        REG_SZ | REG_EXPAND_SZ => {
            let wide: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Value::String(from_wide(&wide))
        }
        _ => Value::Binary(data),
    }
}

fn encode(value: &Value) -> (REG_VALUE_TYPE, Vec<u8>) {
    match value {
        Value::Dword(v) => (REG_DWORD, v.to_le_bytes().to_vec()),
        Value::String(s) => (
            REG_SZ,
            s.encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(|c| c.to_le_bytes())
                .collect(),
        ),
        Value::Binary(b) => (REG_BINARY, b.clone()),
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    use super::*;

    #[test]
    fn reads_well_known_values_without_writing() {
        let v = read_value(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "ProductName",
        )
        .unwrap();
        assert!(matches!(v, Some(Value::String(s)) if s.contains("Windows")));
        assert_eq!(
            read_value(
                HKEY_LOCAL_MACHINE,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "NoSuchValue__"
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn encode_decode_round_trip() {
        for v in [
            Value::Dword(42),
            Value::String("gradual".into()),
            Value::Binary(vec![3, 0, 1]),
        ] {
            let (ty, bytes) = encode(&v);
            assert_eq!(decode(ty, bytes), v);
        }
    }
}
