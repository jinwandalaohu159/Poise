//! 返回给界面的数据结构，按界面的页面分组。界面只负责显示，所有判断都在服务里完成。

mod apps;
mod cleanup;
mod diagnosis;
mod status;

pub use apps::*;
pub use cleanup::*;
pub use diagnosis::*;
pub use status::*;
