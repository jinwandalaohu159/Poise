//! Poise 的纯业务逻辑：不依赖任何系统 API，全部可单元测试。

pub mod action;
pub mod activity;
pub mod classify;
pub mod cleanup;
pub mod diagnose;
pub mod journal;
pub mod known;
pub mod model;
pub mod optimize;
pub mod prefs;
pub mod safety;
pub mod schedule;
