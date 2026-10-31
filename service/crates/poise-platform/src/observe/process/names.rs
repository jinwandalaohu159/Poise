//! 程序的友好名称：可执行文件版本信息里的文件描述或产品名，界面上显示它。
//!
//! 读版本信息要访问磁盘（几毫秒，冷启动时上百毫秒），放在后台线程里：采样从不碰磁盘，
//! 新程序的名称稍后补上（补上之前界面显示文件名）。每个路径只读一次。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crate::win::version_info;

/// 记住的名称超过这么多时，只留下还在运行的程序的（版本号目录、临时安装程序会让路径越来越多）。
const MAX_KNOWN: usize = 4096;

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
    /// 收下后台线程读到的名称（每次采样开始时调用）。后台线程意外结束了的话，没有结果的路径之后重新问
    /// （交给新的线程），不一直记在"已经问过"里。
    pub fn collect(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        loop {
            match worker.answers.try_recv() {
                Ok((path, name)) => {
                    self.asked.remove(&path);
                    self.known.insert(path, name);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    log::warn!("读取程序名称的线程结束了，重新开始");
                    self.worker = None;
                    self.asked.clear();
                    return;
                }
            }
        }
    }

    /// 路径对应的友好名称；还没读到时交给后台线程，先返回 None。
    pub fn name(&mut self, path: &str) -> Option<String> {
        if let Some(name) = self.known.get(path) {
            return name.clone();
        }
        if !self.asked.contains(path) && self.worker().ask.send(path.to_string()).is_ok() {
            self.asked.insert(path.to_string());
        }
        None
    }

    /// 记住的名称太多时（见 [`MAX_KNOWN`]）只留下 `alive` 选中的路径；不多时什么都不做，也不调用 `alive`。
    pub fn trim(&mut self, alive: impl FnOnce() -> HashSet<String>) {
        if self.known.len() <= MAX_KNOWN {
            return;
        }
        let alive = alive();
        self.known.retain(|path, _| alive.contains(path));
    }

    fn worker(&mut self) -> &Worker {
        self.worker.get_or_insert_with(|| {
            let (ask, paths) = mpsc::channel::<String>();
            let (reply, answers) = mpsc::channel();
            std::thread::Builder::new()
                .name("names".into())
                .spawn(move || {
                    for path in paths {
                        let name = display_name(version_info::read(&path));
                        if reply.send((path, name)).is_err() {
                            return;
                        }
                    }
                })
                .expect("创建名称读取线程失败");
            Worker { ask, answers }
        })
    }
}

/// 程序的友好名称：文件描述，其次产品名，再其次原始文件名（去掉 .exe，保留大小写）。
/// 系统组件常把描述写成"Microsoft® Windows® Operating System"，看不出是哪个程序（实测是"小组件"），
/// 这类笼统的说法不用。
fn display_name(v: version_info::VersionStrings) -> Option<String> {
    let stem = v.original_filename.map(|f| {
        let f = f.trim();
        f.strip_suffix(".exe")
            .or_else(|| f.strip_suffix(".EXE"))
            .unwrap_or(f)
            .to_string()
    });
    [v.file_description, v.product_name, stem]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty() && !is_generic(s))
}

/// 只说明是 Windows 的一部分、看不出是哪个程序的名称。
fn is_generic(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("operating system")
        || lower.contains("操作系统")
        || matches!(lower.as_str(), "windows" | "microsoft windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_windows_descriptions_give_way_to_the_file_name() {
        let v = |desc: &str, product: &str, original: Option<&str>| version_info::VersionStrings {
            file_description: Some(desc.into()),
            product_name: Some(product.into()),
            company_name: None,
            original_filename: original.map(Into::into),
        };
        let os = "Microsoft® Windows® Operating System";
        assert_eq!(
            display_name(v(os, os, Some("Widgets.exe"))).as_deref(),
            Some("Widgets")
        );
        assert_eq!(
            display_name(v("Windows Terminal", os, None)).as_deref(),
            Some("Windows Terminal")
        );
        assert_eq!(display_name(v(os, os, None)), None);
    }

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
