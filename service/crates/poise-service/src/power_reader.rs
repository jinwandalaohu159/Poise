//! 电源设置的读取线程：当前电源计划、Poise 写入的值还在不在、电源优化项是否仍是优化状态，都在这里读。
//!
//! 读电源设置要经过系统的电源服务。它忙的时候（开机、登录、刚切换电源计划或模式）每次读取要几十到
//! 几百毫秒，一次检查要读二三十项，实测开机时整轮检查要 2.6 秒——放在调度引擎里，调度和界面请求都得等。
//! 放到这里后，引擎只提交"要读什么"、下一轮取结果，从不等待。写入（改回来、重新应用）仍由引擎做：
//! 变更日志只有一份，而且写入很少发生。

use std::sync::mpsc::{self, Receiver, Sender};

use poise_core::setting::SettingStore;
use poise_platform::settings::store::WinSettingStore;

/// 一次读取：在读取线程里、用它自己的设置存储运行，结果交回引擎。
pub type Job<R> = Box<dyn FnOnce(&dyn SettingStore) -> R + Send>;

/// 读取线程的句柄。丢弃时线程结束。
pub struct PowerReader<R> {
    jobs: Option<Sender<Job<R>>>,
    results: Receiver<R>,
    /// 已提交、结果还没取走的读取。
    pending: usize,
}

impl<R: Send + 'static> PowerReader<R> {
    pub fn start() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job<R>>();
        let (result_tx, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("power".into())
            .spawn(move || {
                let store = WinSettingStore::new(None);
                for job in job_rx {
                    if result_tx.send(job(&store)).is_err() {
                        return;
                    }
                }
            })
            .expect("创建电源设置读取线程失败");
        Self {
            jobs: Some(job_tx),
            results,
            pending: 0,
        }
    }

    /// 上一次提交的读取都已经有结果：可以再提交。电源服务卡住时不越积越多。
    pub fn idle(&self) -> bool {
        self.pending == 0
    }

    /// 提交一次读取（不等待）。
    pub fn submit(&mut self, job: Job<R>) {
        if let Some(tx) = &self.jobs
            && tx.send(job).is_ok()
        {
            self.pending += 1;
        }
    }

    /// 取走已经读完的结果（不等待），按提交的顺序。
    pub fn results(&mut self) -> Vec<R> {
        let done: Vec<R> = self.results.try_iter().collect();
        self.pending = self.pending.saturating_sub(done.len());
        done
    }
}

impl<R> Drop for PowerReader<R> {
    fn drop(&mut self) {
        // 关闭通道，线程读完手头的一项就结束。
        self.jobs.take();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn jobs_run_on_the_reader_thread_and_results_come_back_in_order() {
        let mut reader = PowerReader::start();
        assert!(reader.idle());
        for i in 0..3 {
            reader.submit(Box::new(move |_: &dyn SettingStore| {
                (i, std::thread::current().name().map(str::to_string))
            }));
        }
        assert!(!reader.idle());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < 3 && Instant::now() < deadline {
            got.extend(reader.results());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            got.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(got.iter().all(|(_, name)| name.as_deref() == Some("power")));
        assert!(reader.idle());
    }
}
