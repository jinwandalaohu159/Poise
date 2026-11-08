//! 本机程序之间的网络连接：谁在等谁。浏览器里和本地的大模型服务聊天、前端连本地开发服务器、程序查本地
//! 数据库、访问容器发布到本机的端口——真正干活的是一个用户从没点开过的程序，只能从连接上看出来
//! 它在为正在用的程序干活（调度怎么用见 `poise_core::schedule`）。
//!
//! 只读系统的 TCP 连接表（不打开任何进程，也不看传输的内容）：两端都在本机的已建立连接，监听着那个端口的
//! 一方是服务端。读一次不到一毫秒，最多 [`REFRESH_MS`] 读一次。

mod read;

use std::collections::HashSet;

use poise_core::model::{LocalLink, Millis, Pid};
use read::{ESTABLISHED, Endpoint, LISTEN, Row, read_all};

/// 多久重新读一次。
const REFRESH_MS: Millis = 10_000;

/// 读过的结果，到时间再读。
#[derive(Debug, Default)]
pub struct LocalLinks {
    links: Vec<LocalLink>,
    read_at: Option<Millis>,
}

impl LocalLinks {
    /// 此刻（最近一次读到的）本机程序之间的连接。
    pub fn sample_at(&mut self, at: Millis) -> Vec<LocalLink> {
        if self
            .read_at
            .is_none_or(|t| at.saturating_sub(t) >= REFRESH_MS || at < t)
        {
            self.links = links(&read_all());
            self.read_at = Some(at);
        }
        self.links.clone()
    }
}

/// 从连接表算出（客户端进程，服务端进程）：两端都在本机的已建立连接，对端监听着这个端口的一方是服务端。
fn links(rows: &[Row]) -> Vec<LocalLink> {
    let listening: HashSet<(Pid, u16)> = rows
        .iter()
        .filter(|r| r.state == LISTEN)
        .map(|r| (r.pid, r.local.1))
        .collect();
    let established: std::collections::HashMap<(Endpoint, Endpoint), Pid> = rows
        .iter()
        .filter(|r| r.state == ESTABLISHED)
        .map(|r| ((r.local, r.remote), r.pid))
        .collect();
    let mut out: HashSet<LocalLink> = HashSet::new();
    for r in rows.iter().filter(|r| r.state == ESTABLISHED) {
        // 对端的那一行：本机地址、端口互换。找不到说明对端不在本机。
        let Some(&peer) = established.get(&(r.remote, r.local)) else {
            continue;
        };
        if peer == r.pid || peer == 0 || r.pid == 0 {
            continue;
        }
        if listening.contains(&(peer, r.remote.1)) && !listening.contains(&(r.pid, r.local.1)) {
            out.insert(LocalLink {
                client: r.pid,
                server: peer,
            });
        }
    }
    let mut out: Vec<LocalLink> = out.into_iter().collect();
    out.sort_by_key(|l| (l.client, l.server));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOPBACK: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 127, 0, 0, 1];
    const REMOTE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 1, 2, 3, 4];

    fn row(state: u32, local: u16, remote: ([u8; 16], u16), pid: Pid) -> Row {
        Row {
            state,
            local: (LOOPBACK, local),
            remote,
            pid,
        }
    }

    #[test]
    fn the_side_listening_on_the_port_is_the_server() {
        const BROWSER: Pid = 10;
        const OLLAMA: Pid = 20;
        let rows = [
            row(LISTEN, 11434, ([0; 16], 0), OLLAMA),
            row(ESTABLISHED, 50001, (LOOPBACK, 11434), BROWSER),
            row(ESTABLISHED, 11434, (LOOPBACK, 50001), OLLAMA),
            // 连到外面的连接不算。
            row(ESTABLISHED, 50002, (REMOTE, 443), BROWSER),
        ];
        assert_eq!(
            links(&rows),
            vec![LocalLink {
                client: BROWSER,
                server: OLLAMA
            }]
        );
    }

    #[test]
    fn this_machine_reads_without_error() {
        let mut l = LocalLinks::default();
        let started = std::time::Instant::now();
        let found = l.sample_at(0);
        println!(
            "{} 条本机连接，用时 {:?}：{found:?}",
            found.len(),
            started.elapsed()
        );
        assert!(l.read_at.is_some());
    }
}
