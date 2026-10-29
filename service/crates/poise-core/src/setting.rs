//! 设置：一个可读写、可还原的位置（注册表值、用户目录下的配置文件、电源计划中的设置）与它的取值，
//! 以及平台层实现的读写接口。系统优化项、硬件模式、游戏的系统设置都通过它修改，变更日志按它记录原值。

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hive {
    LocalMachine,
    /// 当前交互用户（服务以 SYSTEM 运行时，由平台层解析到该用户的配置单元）。
    CurrentUser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Value {
    Dword(u32),
    String(String),
    Binary(Vec<u8>),
}

/// 用户目录下的配置文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserFile {
    /// `%USERPROFILE%\.wslconfig`
    WslConfig,
}

/// 一个可读写、可删除的设置位置。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Registry {
        hive: Hive,
        key: String,
        name: String,
    },
    Ini {
        file: UserFile,
        section: String,
        key: String,
    },
    /// 电源计划中的某个设置（GUID 字符串）。
    PowerSetting {
        /// 所属的电源计划；None 表示当前计划。变更日志里记的都是具体的计划（见 [`SettingStore::resolve`]）：
        /// 切换计划之后，还原仍写回当初修改的那个计划，不会把旧计划的原值写进新计划。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scheme: Option<String>,
        subgroup: String,
        setting: String,
        ac: bool,
    },
}

impl Target {
    /// 当前电源计划中的某个设置。
    pub fn power(subgroup: &str, setting: &str, ac: bool) -> Self {
        Self::PowerSetting {
            scheme: None,
            subgroup: subgroup.into(),
            setting: setting.into(),
            ac,
        }
    }

    /// 是否是同一个位置。注册表的键名和值名、INI 的节名和键名、GUID 都不区分大小写
    /// （路径来自文件选择框或不同版本，大小写可能不同）。
    pub fn same_place(&self, other: &Self) -> bool {
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        match (self, other) {
            (
                Self::Registry { hive, key, name },
                Self::Registry {
                    hive: h,
                    key: k,
                    name: n,
                },
            ) => hive == h && eq(key, k) && eq(name, n),
            (
                Self::Ini { file, section, key },
                Self::Ini {
                    file: f,
                    section: s,
                    key: k,
                },
            ) => file == f && eq(section, s) && eq(key, k),
            (
                Self::PowerSetting {
                    scheme,
                    subgroup,
                    setting,
                    ac,
                },
                Self::PowerSetting {
                    scheme: sc,
                    subgroup: sg,
                    setting: st,
                    ac: a,
                },
            ) => {
                let same_scheme = match (scheme, sc) {
                    (Some(x), Some(y)) => eq(x, y),
                    (None, None) => true,
                    _ => false,
                };
                same_scheme && eq(subgroup, sg) && eq(setting, st) && ac == a
            }
            _ => false,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StoreError {
    #[error("没有权限")]
    AccessDenied,
    #[error("不支持的位置")]
    Unsupported,
    #[error("{0}")]
    Io(String),
}

/// 平台层实现的设置读写。
pub trait SettingStore {
    fn read(&self, target: &Target) -> Result<Option<Value>, StoreError>;
    fn write(&self, target: &Target, value: &Value) -> Result<(), StoreError>;
    fn delete(&self, target: &Target) -> Result<(), StoreError>;

    /// 把相对的位置换成具体的位置（例如"当前电源计划"换成具体是哪个计划），变更日志按它记录。默认不变。
    fn resolve(&self, target: &Target) -> Result<Target, StoreError> {
        Ok(target.clone())
    }

    /// 开始一批写入：批内的修改可以推迟到 [`SettingStore::end_batch`] 时一起生效
    /// （例如电源计划只重新激活一次，而不是每写一项激活一次）。默认无操作。
    fn begin_batch(&self) {}

    /// 结束一批写入，让推迟的修改生效。默认无操作。
    fn end_batch(&self) {}

    /// 结束一批写入，但不在这里让推迟的修改生效，而是返回是否还有待生效的修改：
    /// 生效很慢时（重新激活电源计划约 200 毫秒），调用方可以放到别的线程里做。
    fn end_batch_deferred(&self) -> bool {
        self.end_batch();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_place_ignores_case_but_not_the_power_plan() {
        let reg = |key: &str, name: &str| Target::Registry {
            hive: Hive::CurrentUser,
            key: key.into(),
            name: name.into(),
        };
        assert!(reg(r"Software\A", r"D:\Game.exe").same_place(&reg(r"software\a", r"d:\game.exe")));
        assert!(!reg("a", "x").same_place(&reg("a", "y")));
        let power = |scheme: Option<&str>| Target::PowerSetting {
            scheme: scheme.map(Into::into),
            subgroup: "S".into(),
            setting: "X".into(),
            ac: true,
        };
        assert!(power(Some("ABC")).same_place(&power(Some("abc"))));
        assert!(!power(Some("abc")).same_place(&power(Some("def"))));
        assert!(!power(None).same_place(&power(Some("abc"))));
    }
}
