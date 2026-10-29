//! 程序的友好名称：可执行文件版本信息里的文件描述或产品名，界面上显示它。
//!
//! 读版本信息要访问磁盘（几毫秒，冷启动时上百毫秒），放在后台线程里：采样从不碰磁盘，
//! 新程序的名称稍后补上（补上之前界面显示文件名）。每个路径只读一次。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::win::version_info;

#[derive(Default)]
pub struct NameResolver {
    /// 已经读到的名称（None 表示文件里没有）。
    known: HashMap<String, Option<String>>,
    /// 已经交给后台线程、还没有结果的路径。
    asked: HashSet<String>,
    worker: Option<Worker>,
}

struct Worker {
    ask: Sender<String>,
    answers: Receiver<(String, Option<String>)>,
}

impl NameResolver {
    /// 收下后台线程读到的名称（每次采样开始时调用）。
    pub fn collect(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        while let Ok((path, name)) = worker.answers.try_recv() {
            self.asked.remove(&path);
            self.known.insert(path, name);
        }
    }

    /// 路径对应的友好名称；还没读到时交给后台线程，先返回 None。
    pub fn name(&mut self, path: &str) -> Option<String> {
        if let Some(name) = self.known.get(path) {
            return name.clone();
        }
        if self.asked.insert(path.to_string()) {
            let _ = self.worker().ask.send(path.to_string());
        }
        None
    }

    fn worker(&mut self) -> &Worker {
        self.worker.get_or_insert_with(|| {
            let (ask, paths) = mpsc::channel::<String>();
            let (reply, answers) = mpsc::channel();
            std::thread::Builder::new()
                .name("names".into())
                .spawn(move || {
                    for path in paths {
                        let v = version_info::read(&path);
                        if reply
                            .send((path, v.file_description.or(v.product_name)))
                            .is_err()
                        {
                            return;
                        }
                    }
                })
                .expect("创建名称读取线程失败");
            Worker { ask, answers }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_arrive_later_without_blocking() {
        let mut r = NameResolver::default();
        let me = std::env::current_exe().unwrap();
        let path = me.to_string_lossy().to_lowercase();
        assert_eq!(r.name(&path), None);
        for _ in 0..100 {
            r.collect();
            if r.known.contains_key(&path) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("名称没有读到");
    }
}
