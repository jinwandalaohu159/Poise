//! 每轮采样各部分的开销（只读，不执行任何动作），用这台电脑上真实的进程表测量。都是手动运行的测量，
//! 不参与平时的测试：
//!
//! ```text
//! cargo test --release -p poise-platform --test cycle_cost -- --ignored --nocapture
//! ```
//!
//! 测某个正在渲染的进程的显卡占用：再加环境变量 `GPU_PID=<进程号>`。

use std::time::{Duration, Instant};

use poise_core::activity::{ActivityConfig, ActivityTracker};
use poise_core::classify::{Classifier, ClassifyConfig, ClassifyInput};
use poise_core::model::SessionState;
use poise_core::prefs::{Mode, Preferences};
use poise_core::schedule::{CoreLayout, ScheduleConfig, ScheduleInput, Scheduler};
use poise_platform::observe::process::snapshot::Snapshotter;
use poise_platform::observe::topology::CpuTopology;

#[test]
#[ignore]
fn measure_one_cycle() {
    let topology = CpuTopology::detect();
    let mut snapshotter = Snapshotter::default();
    let mut activity = ActivityTracker::new(ActivityConfig::default());
    let mut classifier = Classifier::new(ClassifyConfig::default());
    let mut scheduler = Scheduler::new(ScheduleConfig {
        cores: CoreLayout::new(&topology.domain_threads()),
        ..Default::default()
    });
    let prefs = Preferences::default();
    let session = SessionState {
        session_id: 1,
        ..Default::default()
    };
    let started = Instant::now();
    let mut totals = [Duration::ZERO; 4];
    let rounds = 20;
    let mut processes = 0;
    for round in 0..rounds + 1 {
        std::thread::sleep(Duration::from_millis(200));
        let now = started.elapsed().as_millis() as u64;
        let t = Instant::now();
        let snap = snapshotter.take(now).unwrap();
        let take = t.elapsed();
        let t = Instant::now();
        activity.update(&snap);
        let observe = t.elapsed();
        let t = Instant::now();
        let classes = classifier.classify_all(&ClassifyInput {
            snapshot: &snap,
            session: &session,
            activity: &activity,
            preferences: &prefs,
            own_pids: &[],
            foreground_gpu: None,
            active_games: &[],
            session_fresh: true,
        });
        let classify = t.elapsed();
        let t = Instant::now();
        let plan = scheduler.plan(&ScheduleInput {
            snapshot: &snap,
            classes: &classes,
            activity: &activity,
            enabled: true,
            user_mode: Mode::Balanced,
            game_mode: None,
            on_battery: false,
            hot: false,
        });
        let schedule = t.elapsed();
        processes = snap.processes.len();
        // 第一轮建缓存，不算。
        if round > 0 {
            for (total, d) in totals.iter_mut().zip([take, observe, classify, schedule]) {
                *total += d;
            }
        }
        std::hint::black_box(plan);
    }
    let avg = |d: Duration| d.as_micros() / rounds as u128;
    println!(
        "{processes} 个进程，每轮平均（微秒）：采集 {}，活动 {}，分类 {}，调度 {}",
        avg(totals[0]),
        avg(totals[1]),
        avg(totals[2]),
        avg(totals[3])
    );
}

#[test]
#[ignore]
fn measure_process_table() {
    use poise_platform::observe::process::enumerate::ProcessTable;
    use poise_platform::observe::system::thread_cpu_time;
    let mut table = ProcessTable::default();
    table.read().unwrap();
    for pause in [0u64, 200] {
        let (mut wall, mut cpu) = (Duration::ZERO, Duration::ZERO);
        let mut threads = 0;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(pause));
            let (t, c) = (Instant::now(), thread_cpu_time());
            let rows = table.read().unwrap();
            wall += t.elapsed();
            cpu += thread_cpu_time() - c;
            threads = rows.iter().map(|r| r.metrics.threads).sum::<u32>();
        }
        println!(
            "间隔 {pause} ms：进程表每次 {} 微秒（处理器 {} 微秒），{threads} 个线程",
            wall.as_micros() / 20,
            cpu.as_micros() / 20
        );
    }
}

#[test]
#[ignore]
fn measure_system_metrics() {
    use poise_platform::observe::system;
    let time = |name: &str, f: &dyn Fn()| {
        f();
        let t = Instant::now();
        for _ in 0..50 {
            f();
        }
        println!("{name}：每次 {} 微秒", t.elapsed().as_micros() / 50);
    };
    time("各逻辑处理器的时间", &|| {
        std::hint::black_box(poise_platform::observe::processors::read_processor_times());
    });
    let times = poise_platform::observe::processors::read_processor_times();
    time("整机指标（处理器时间已读好）", &|| {
        std::hint::black_box(system::read_metrics(&times));
    });
    time("on_battery", &|| {
        std::hint::black_box(system::on_battery());
    });
}

/// 测自己（没在渲染）的开销：开始测一次、之后每次读数。
#[test]
#[ignore]
fn measure_gpu_meter() {
    use poise_platform::observe::gpu::GpuMeter;
    use poise_platform::observe::system::thread_cpu_time;
    let pid = std::process::id();
    let mut meter = GpuMeter::default();
    let (t, c) = (Instant::now(), thread_cpu_time());
    meter.usage_3d(pid);
    println!(
        "打开：{} 微秒（处理器 {} 微秒）",
        t.elapsed().as_micros(),
        (thread_cpu_time() - c).as_micros()
    );
    let (mut wall, mut cpu) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(200));
        let (t, c) = (Instant::now(), thread_cpu_time());
        meter.usage_3d(pid);
        wall += t.elapsed();
        cpu += thread_cpu_time() - c;
    }
    println!(
        "每次读数：{} 微秒（处理器 {} 微秒）",
        wall.as_micros() / 10,
        cpu.as_micros() / 10
    );
}

/// 正在渲染的进程（`GPU_PID`）的显卡占用读数，可以和任务管理器的"GPU"列对照。没给进程号时跳过。
#[test]
#[ignore]
fn measure_gpu_of_a_rendering_process() {
    use poise_platform::observe::gpu::GpuMeter;
    use poise_platform::observe::system::thread_cpu_time;
    let Some(pid) = std::env::var("GPU_PID")
        .ok()
        .and_then(|p| p.parse::<u32>().ok())
    else {
        return;
    };
    let (t, c) = (Instant::now(), thread_cpu_time());
    let mut meter = GpuMeter::default();
    meter.usage_3d(pid);
    println!(
        "开始测：{} 微秒（处理器 {} 微秒）",
        t.elapsed().as_micros(),
        (thread_cpu_time() - c).as_micros()
    );
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(1000));
        let t = Instant::now();
        let v = meter.usage_3d(pid);
        println!("占用 {v:?}（{} 微秒）", t.elapsed().as_micros());
    }
}
