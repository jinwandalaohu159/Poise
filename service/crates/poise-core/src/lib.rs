//! Poise 的纯业务逻辑：不依赖任何系统 API，全部可单元测试。

pub mod action;
pub mod activity;
pub mod app_groups;
pub mod auto_mode;
pub mod classify;
pub mod cleanup;
pub mod diagnose;
pub mod fast_hash;
pub mod journal;
pub mod known;
pub mod model;
pub mod optimize;
pub mod power_settings;
pub mod prefs;
pub mod safety;
pub mod schedule;
pub mod seen_apps;
pub mod setting;
