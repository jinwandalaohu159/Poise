//! 连着的界面和托盘：它们是 Poise 自己的进程，永不触碰。同一个进程可以有几条连接（界面的实时数据和慢请求
//! 各一条，托盘的一键清理单独一条），最后一条断开才算它走了——否则托盘的清理连接一关，服务就当托盘代理
//! 退出了而暂停。

use std::collections::HashMap;

use poise_core::model::Pid;

#[derive(Default)]
pub(super) struct Clients(HashMap<Pid, u32>);

impl Clients {
    /// 连上一条（只接受安装目录里的程序连接，见 `ipc_server`）。
    pub fn connected(&mut self, pid: Pid) {
        *self.0.entry(pid).or_default() += 1;
    }

    /// 断开一条；返回这个进程的连接是不是都断开了。
    pub fn disconnected(&mut self, pid: Pid) -> bool {
        let Some(count) = self.0.get_mut(&pid) else {
            return false;
        };
        *count -= 1;
        if *count > 0 {
            return false;
        }
        self.0.remove(&pid);
        true
    }

    /// Poise 自己的进程：服务本身与连着的界面、托盘。
    pub fn own_pids(&self) -> Vec<Pid> {
        std::iter::once(std::process::id())
            .chain(self.0.keys().copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_leaves_when_its_last_connection_closes() {
        let mut c = Clients::default();
        c.connected(7);
        c.connected(7);
        assert!(c.own_pids().contains(&7));
        assert!(!c.disconnected(7));
        assert!(c.own_pids().contains(&7));
        assert!(c.disconnected(7));
        assert!(!c.own_pids().contains(&7));
        // 没连过的不算。
        assert!(!c.disconnected(8));
        assert!(c.own_pids().contains(&std::process::id()));
    }
}
