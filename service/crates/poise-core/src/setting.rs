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
        /// 按用户的位置属于哪个用户（SID）；None 表示当前用户。变更日志里记的都是具体的用户（见
        /// [`SettingStore::resolve`]）：换了用户（快速切换用户、另一个账户登录）之后，还原仍写回当初修改的那个
        /// 用户，不会把一个用户的原值写进另一个用户的设置。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// 用户目录下的配置文件（总是按用户的）。
    Ini {
        file: UserFile,
        section: String,
        key: String,
        /// 按用户的位置属于哪个用户（SID）；None 表示当前用户。变更日志里记的都是具体的用户（见
        /// [`SettingStore::resolve`]）：换了用户（快速切换用户、另一个账户登录）之后，还原仍写回当初修改的那个
        /// 用户，不会把一个用户的原值写进另一个用户的设置。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
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
    /// 当前用户（`Hive::CurrentUser`）或本机的注册表值。
    pub fn registry(hive: Hive, key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Registry {
            hive,
            key: key.into(),
            name: name.into(),
            user: None,
        }
    }

    /// 当前用户目录下的配置文件中的一项。
    pub fn ini(file: UserFile, section: impl Into<String>, key: impl Into<String>) -> Self {
        Self::Ini {
            file,
            section: section.into(),
            key: key.into(),
            user: None,
        }
    }

    /// 按用户的位置（当前用户的注册表、用户目录下的文件）。
    pub fn per_user(&self) -> bool {
        matches!(
            self,
            Self::Registry {
                hive: Hive::CurrentUser,
                ..
            } | Self::Ini { .. }
        )
    }

    /// 按用户的位置写明的用户（SID）；没写明、不是按用户的位置时为 None。
    pub fn user(&self) -> Option<&str> {
        match self {
            Self::Registry { user, .. } | Self::Ini { user, .. } => user.as_deref(),
            Self::PowerSetting { .. } => None,
        }
    }

    /// 同一位置、不写明用户（交给 [`SettingStore::resolve`] 换成当前用户）。
    pub fn any_user(mut self) -> Self {
        if let Self::Registry { user, .. } | Self::Ini { user, .. } = &mut self {
            *user = None;
        }
        self
    }

    /// 写明是哪个用户的（只对按用户的位置有效，其余不变）。
    pub fn for_user(mut self, sid: &str) -> Self {
        if self.per_user()
            && let Self::Registry { user, .. } | Self::Ini { user, .. } = &mut self
        {
            *user = Some(sid.to_string());
        }
        self
    }

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
        // 写明的计划、用户都要相同；一边写明、一边没写明不算同一个位置（解析之后再比较）。
        let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(x), Some(y)) => eq(x, y),
            (None, None) => true,
            _ => false,
        };
        match (self, other) {
            (
                Self::Registry {
                    hive,
                    key,
                    name,
                    user,
                },
                Self::Registry {
                    hive: h,
                    key: k,
                    name: n,
                    user: u,
                },
            ) => hive == h && eq(key, k) && eq(name, n) && same(user, u),
            (
                Self::Ini {
                    file,
                    section,
                    key,
                    user,
                },
                Self::Ini {
                    file: f,
                    section: s,
                    key: k,
                    user: u,
                },
            ) => file == f && eq(section, s) && eq(key, k) && same(user, u),
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
            ) => same(scheme, sc) && eq(subgroup, sg) && eq(setting, st) && ac == a,
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

    /// 把相对的位置换成具体的位置（"当前电源计划"换成具体是哪个计划，"当前用户"换成具体是哪个用户），
    /// 变更日志按它记录。默认不变。
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
        let reg = |key: &str, name: &str| Target::registry(Hive::CurrentUser, key, name);
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

    #[test]
    fn per_user_places_belong_to_one_user() {
        let mine = Target::registry(Hive::CurrentUser, "K", "V").for_user("S-1-5-21-1");
        assert_eq!(mine.user(), Some("S-1-5-21-1"));
        assert!(
            mine.same_place(&Target::registry(Hive::CurrentUser, "k", "v").for_user("s-1-5-21-1"))
        );
        assert!(
            !mine.same_place(&Target::registry(Hive::CurrentUser, "K", "V").for_user("S-1-5-21-2"))
        );
        assert!(!mine.same_place(&Target::registry(Hive::CurrentUser, "K", "V")));
        // 本机的位置、电源设置不属于哪个用户。
        assert_eq!(
            Target::registry(Hive::LocalMachine, "K", "V")
                .for_user("S-1")
                .user(),
            None
        );
        assert_eq!(Target::power("S", "X", true).for_user("S-1").user(), None);
        assert_eq!(
            Target::ini(UserFile::WslConfig, "wsl2", "memory")
                .for_user("S-1")
                .user(),
            Some("S-1")
        );
    }
}
