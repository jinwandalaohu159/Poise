//! 服务一轮采样每一步的开销，用这台电脑上真实的进程表测量。观察模式：只判断、只记录，不执行任何动作，
//! 也不修改任何设置（不影响已安装的 Poise）。手动运行：
//!
//! ```text
//! cargo test --release -p poise-service tick_cost -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use poise_core::model::SessionState;
use poise_logbook::Logbook;
use poise_platform::observe::system;

use super::Engine;
use crate::config::{RunConfig, RunMode};

#[test]
#[ignore]
fn measure_engine_tick() {
    let dir = std::env::temp_dir().join(format!("poise-tick-cost-{}", std::process::id()));
    let cfg = RunConfig {
        mode: RunMode::Console,
        dry_run: true,
        data_dir: dir.clone(),
    };
    let logbook = Logbook::open(&dir);
    let mut engine = Engine::new(cfg, logbook.clone()).expect("创建引擎失败");
    // 假装托盘代理在线：按时上报会话状态，前台是这个测试进程。
    let session = SessionState {
        session_id: system::active_console_session(),
        foreground_pid: Some(std::process::id()),
        ..Default::default()
    };
    let (warmup, rounds) = (3, 30);
    let mut steps: Vec<(&'static str, Duration, Duration)> = Vec::new();
    let (mut wall, mut cpu) = (Duration::ZERO, Duration::ZERO);
    let started = Instant::now();
    for i in 0..warmup + rounds {
        let now = engine.now();
        engine.session.report(session.clone(), 1, now);
        std::thread::sleep(Duration::from_millis(200));
        let cost = engine.tick(engine.sampler().sample());
        if i < warmup {
            continue;
        }
        wall += cost.total.wall;
        cpu += cost.total.cpu;
        for (k, (name, spent)) in cost.steps.iter().enumerate() {
            if steps.len() <= k {
                steps.push((name, Duration::ZERO, Duration::ZERO));
            }
            steps[k].1 += spent.wall;
            steps[k].2 += spent.cpu;
        }
    }
    let avg = |d: Duration| d.as_micros() / rounds;
    println!(
        "{} 个进程，{} 轮（{} 秒），每轮平均（微秒）：总共 {}（处理器 {}）",
        engine
            .view
            .as_ref()
            .map_or(0, |v| v.snapshot.processes.len()),
        rounds,
        started.elapsed().as_secs(),
        avg(wall),
        avg(cpu)
    );
    for (name, w, c) in steps {
        println!("  {name}：{}（处理器 {}）", avg(w), avg(c));
    }
    engine.shutdown(false);
    logbook.flush();
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
}
