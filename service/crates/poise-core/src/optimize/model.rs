//! 优化项的数据模型：每一项都是"把某个设置改成某个值"，因此天然可还原。

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
    /// 当前电源计划中的某个设置（GUID 字符串）。
    PowerSetting {
        subgroup: String,
        setting: String,
        ac: bool,
    },
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

    /// 开始一批写入：批内的修改可以推迟到 [`SettingStore::end_batch`] 时一起生效
    /// （例如电源计划只重新激活一次，而不是每写一项激活一次）。默认无操作。
    fn begin_batch(&self) {}

    /// 结束一批写入，让推迟的修改生效。默认无操作。
    fn end_batch(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// 电源计划里影响发热与性能的设置。
    Power,
    Explorer,
    StartupBackground,
    Memory,
    /// 右键菜单插件：逐个开关，一键优化不触碰。
    ContextMenu,
    /// 资源管理器导航栏里的第三方入口：逐个开关。
    NavigationPane,
    /// 开机启动项：逐个开关。
    StartupApps,
}

/// 界面文案：静态项用本地化 key，动态项（插件名）直接用原文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Text {
    Key(String),
    Plain(String),
}

/// 修改后什么时候生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TakesEffect {
    #[default]
    Immediately,
    NextFolderWindow,
    NextSignIn,
    NextRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationItem {
    pub id: String,
    pub group: Group,
    pub title: Text,
    pub description: Text,
    pub target: Target,
    /// 优化后的值。
    pub optimized: Value,
    /// 如何判断"已经是优化状态"。
    #[serde(default)]
    pub matcher: Matcher,
    /// 是否包含在"一键优化"里。
    pub in_one_click: bool,
    /// 可能拖慢右键或资源管理器（仅提示，不替用户关闭）。
    pub slow_hint: bool,
    pub takes_effect: TakesEffect,
}

/// 判断当前值是否已是优化状态的规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matcher {
    /// 与优化值完全相同。
    #[default]
    Exact,
    /// 只要值存在就算（例如插件屏蔽列表里出现了这个 CLSID）。
    Present,
    /// 二进制值首字节为奇数（任务管理器的启动项"已禁用"标记，后面附带时间戳）。
    FirstByteOdd,
    /// 数值不超过给定值即可（例如最小处理器状态只要不强制高频就算已优化）。
    AtMost(u32),
}

impl Matcher {
    pub fn is_optimized(self, current: Option<&Value>, optimized: &Value) -> bool {
        match (self, current) {
            (_, None) => false,
            (Self::Exact, Some(v)) => v == optimized,
            (Self::Present, Some(_)) => true,
            (Self::FirstByteOdd, Some(Value::Binary(b))) => b.first().is_some_and(|x| x % 2 == 1),
            (Self::FirstByteOdd, Some(_)) => false,
            (Self::AtMost(limit), Some(Value::Dword(v))) => *v <= limit,
            (Self::AtMost(_), Some(_)) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Optimized,
    NotOptimized,
    /// 读不到（没有权限等）。
    Unknown,
}
