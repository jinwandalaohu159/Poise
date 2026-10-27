//! 进程的静态属性：路径、关键进程标记、保护级别、友好名称。
//! 每个进程只在第一次见到时读取一次，之后走缓存。

use std::collections::HashMap;
use std::ffi::c_void;

use poise_core::model::{ProcessKey, Protection};
use windows::Win32::System::Threading::{
    GetProcessInformation, IsProcessCritical, PROCESS_PROTECTION_LEVEL_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, ProcessProtectionLevelInfo,
};
use windows::core::BOOL;

use crate::win::handle::{OwnedHandle, image_path_of, open_process};
use crate::win::version_info;

const PROTECTION_LEVEL_NONE: u32 = 0xFFFF_FFFE;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticFacts {
    pub image_path: Option<String>,
    pub display_name: Option<String>,
    pub is_critical: bool,
    pub protection: Protection,
}

#[derive(Default)]
pub struct FactsCache {
    by_key: HashMap<ProcessKey, StaticFacts>,
    names_by_path: HashMap<String, Option<String>>,
}

impl FactsCache {
    pub fn get(&mut self, key: ProcessKey) -> StaticFacts {
        if let Some(f) = self.by_key.get(&key) {
            return f.clone();
        }
        let facts = self.load(key);
        self.by_key.insert(key, facts.clone());
        facts
    }

    /// 丢弃已退出进程的缓存。
    pub fn retain(&mut self, alive: impl Fn(&ProcessKey) -> bool) {
        self.by_key.retain(|k, _| alive(k));
    }

    fn load(&mut self, key: ProcessKey) -> StaticFacts {
        if key.pid <= 4 {
            return StaticFacts {
                is_critical: true,
                ..Default::default()
            };
        }
        let Ok(handle) = open_process(key.pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            // 打不开的进程一律按受保护处理：宁可不碰，也不冒险。
            return StaticFacts {
                protection: Protection::Light,
                ..Default::default()
            };
        };
        let image_path = image_path_of(&handle);
        let display_name = image_path.as_ref().and_then(|p| {
            self.names_by_path
                .entry(p.clone())
                .or_insert_with(|| {
                    let v = version_info::read(p);
                    v.file_description.or(v.product_name)
                })
                .clone()
        });
        StaticFacts {
            image_path: image_path.map(|p| p.to_lowercase()),
            display_name,
            is_critical: is_critical(&handle),
            protection: protection(&handle),
        }
    }
}

fn is_critical(h: &OwnedHandle) -> bool {
    let mut critical = BOOL(0);
    // SAFETY: 输出参数为本地变量。
    unsafe { IsProcessCritical(h.raw(), &mut critical) }.is_ok() && critical.as_bool()
}

fn protection(h: &OwnedHandle) -> Protection {
    let mut info = PROCESS_PROTECTION_LEVEL_INFORMATION::default();
    // SAFETY: 结构体大小与传入长度一致。
    let ok = unsafe {
        GetProcessInformation(
            h.raw(),
            ProcessProtectionLevelInfo,
            &mut info as *mut _ as *mut c_void,
            size_of::<PROCESS_PROTECTION_LEVEL_INFORMATION>() as u32,
        )
    };
    match ok {
        Ok(()) if info.ProtectionLevel.0 == PROTECTION_LEVEL_NONE => Protection::None,
        Ok(()) => Protection::Light,
        // 读不到保护级别时保守处理。
        Err(_) => Protection::Light,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_plain_and_has_a_path() {
        let me = std::process::id();
        let created = crate::observe::process::enumerate::ProcessTable::default()
            .read()
            .unwrap()
            .into_iter()
            .find(|p| p.key.pid == me)
            .unwrap()
            .key;
        let f = FactsCache::default().get(created);
        assert!(f.image_path.is_some_and(|p| p.ends_with(".exe")));
        assert_eq!(f.protection, Protection::None);
        assert!(!f.is_critical);
    }
}
