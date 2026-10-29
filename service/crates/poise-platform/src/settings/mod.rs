//! 持久设置的读写（[`store`]：注册表值、用户目录下的配置文件、电源计划中的设置），
//! 电源计划的底层接口（[`power_plan`]），以及动态优化项的枚举（[`dynamic_items`]）。

pub mod dynamic_items;
pub mod ini;
pub mod power_plan;
pub mod store;
pub mod user;
