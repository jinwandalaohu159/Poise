//! 与界面（C#）之间的契约样本。
//!
//! 这里生成的 JSON 存在 `tests/fixtures/`，界面的测试（Poise.Core.Tests）读取同一批文件反序列化，
//! 两边任何一方改了字段名、枚举值或可空性，都会有一边的测试失败。
//! 修改数据结构后用 `POISE_UPDATE_FIXTURES=1 cargo test -p poise-protocol` 重新生成，再跑界面测试。

use std::path::PathBuf;

use poise_core::cleanup::{
    Change, GrowthMetric, Issue, ProcessGroup, Remedy, TotalChange, TotalItem,
};
use poise_core::diagnose::{Health, HealthCode, HistoryPoint, Level, StutterCause, StutterReport};
use poise_core::power_settings::PowerSettingKind;
use poise_core::prefs::{AppPreference, GameMode, Mode};
use poise_core::schedule::{Cause, DemandReason, Pressure, Verdict};
use poise_protocol::{
    Adjustment, AdvisoryDto, AppEntry, AppKind, AppProcess, AppSchedule, CleanupOverview,
    CleanupResult, CleanupTrigger, CoreDomainDto, CoreMap, CoreSpan, CoreUse, CpuDto, Diagnosis,
    DiskDto, DiskHealthDto, DriveSpace, ErrorCode, Event, EventEntry, EventPage, FanDto,
    HardwareInfo, LastCleanup, MemoryDto, Metrics, PlacedProgram, PriorityLevel, ProcessorDto,
    ProgramRef, RecordProgram, ResponseEnvelope, ScheduleState, SessionRecord, SinceBoot, Status,
    Trend, VolumeDto,
};
use serde::Serialize;

const GB: u64 = 1024 * 1024 * 1024;

/// 所有可选字段都有值。
fn full_status() -> Status {
    Status {
        health: Health {
            code: HealthCode::StorageLow,
            level: Level::Notice,
            game: Some("League of Legends".into()),
            cleanable: 2,
            freeable_bytes: GB,
        },
        mode: Mode::Quiet,
        effective_mode: Mode::Full,
        game_mode: GameMode::Vendor,
        auto_clean: false,
        fan_boost: 60,
        smart_schedule: true,
        metrics: Metrics {
            cpu_pct: 12.5,
            cpu_temp: Some(71.0),
            gpu_temp: Some(58.0),
            gpu_pct: Some(33.0),
            mem_used_pct: 63.0,
            mem_used_bytes: 20 * GB,
            mem_total_bytes: 32 * GB,
            cpu_mhz: 2300,
            on_battery: true,
        },
        hardware: HardwareInfo {
            profile: "Dell G15 5520".into(),
            thermal_control: true,
            fan_boost_supported: true,
            vendor_mode: Some("dell_g_mode".into()),
            fans: vec![
                FanDto {
                    rpm: 2400,
                    max_rpm: 4800,
                },
                FanDto {
                    rpm: 2600,
                    max_rpm: 4800,
                },
            ],
            disks: vec![DiskDto {
                name: "WD SN750 2TB".into(),
                size_bytes: Some(2000 * GB),
                temp_c: Some(45),
                health: DiskHealthDto::Warning,
                wear_pct: Some(3),
                volumes: vec![VolumeDto {
                    letter: "C".into(),
                    free_bytes: 12 * GB,
                    total_bytes: 500 * GB,
                }],
            }],
            system_drive: Some(DriveSpace {
                free_bytes: 12 * GB,
                total_bytes: 500 * GB,
            }),
            memory: Some(MemoryDto {
                total_bytes: 32 * GB,
                speed_mhz: 4800,
                module_bytes: vec![16 * GB, 16 * GB],
            }),
            cpu: Some(CpuDto {
                performance_cores: 6,
                performance_threads: 12,
                efficiency_cores: 8,
                efficiency_threads: 8,
            }),
        },
        adjusted_processes: 7,
        deferred_cleanup: 1,
        dry_run: true,
        last_cleanup: Some(LastCleanup {
            at_unix_ms: 1_800_000_000_000,
            trigger: CleanupTrigger::Deferred,
            freed_bytes: 2 * GB,
            handled: 3,
        }),
    }
}

/// 所有可选字段都为空（刚启动、读不到硬件的机器）。
fn minimal_status() -> Status {
    let mut s = full_status();
    s.health = Health {
        code: HealthCode::Smooth,
        level: Level::Good,
        game: None,
        cleanable: 0,
        freeable_bytes: 0,
    };
    s.metrics.cpu_temp = None;
    s.metrics.gpu_temp = None;
    s.metrics.gpu_pct = None;
    s.hardware = HardwareInfo {
        profile: "Generic".into(),
        thermal_control: false,
        fan_boost_supported: false,
        vendor_mode: None,
        fans: vec![],
        disks: vec![DiskDto {
            name: "Disk".into(),
            size_bytes: None,
            temp_c: None,
            health: DiskHealthDto::Unknown,
            wear_pct: None,
            volumes: vec![],
        }],
        system_drive: None,
        memory: None,
        cpu: None,
    };
    s.smart_schedule = false;
    s.last_cleanup = None;
    s
}

fn cleanup_result() -> CleanupResult {
    CleanupResult {
        freed_bytes: 1100 * 1024 * 1024,
        handled: vec!["Windows 音频设备图形隔离".into()],
        failed: vec!["Example".into()],
        deferred: 1,
        advisories: vec![
            AdvisoryDto::HeavyApp {
                name: "微信".into(),
                memory_bytes: 2 * GB,
            },
            AdvisoryDto::RestartRecommended,
            AdvisoryDto::ManualPending { count: 2 },
        ],
    }
}

fn diagnosis() -> Diagnosis {
    Diagnosis {
        report: StutterReport {
            cause: StutterCause::BackgroundCpu {
                name: "OneDrive".into(),
                path: Some(r"c:\program files\microsoft onedrive\onedrive.exe".into()),
                share_pct: 35.0,
            },
            cpu_pct: 92.0,
            mem_used_pct: 70.0,
            cpu_temp: Some(82.0),
            throttling: false,
            top_cpu: vec![("OneDrive".into(), 35.0), ("Chrome".into(), 4.5)],
            top_memory: vec![("Chrome".into(), 3 * GB)],
        },
        smart_schedule: true,
    }
}

/// 走势：大小核处理器的点与读不到温度的点。
fn trend() -> Trend {
    Trend {
        window_secs: 300,
        points: vec![
            HistoryPoint {
                secs_ago: 4.5,
                cpu_pct: 40.0,
                performance_pct: Some(64.0),
                efficiency_pct: Some(18.5),
                mem_pct: 69.0,
                cpu_temp: Some(75.0),
            },
            HistoryPoint {
                secs_ago: 2.0,
                cpu_pct: 92.0,
                performance_pct: None,
                efficiency_pct: None,
                mem_pct: 70.0,
                cpu_temp: None,
            },
        ],
    }
}

/// 程序偏好页：各种调度状态各一个。
fn apps() -> Vec<AppEntry> {
    let app = |name: &str, preference: AppPreference, schedule: Option<AppSchedule>| AppEntry {
        pid: schedule.map(|_| 100),
        name: name.into(),
        path: format!("c:/apps/{name}.exe"),
        preference,
        running: schedule.is_some(),
        is_game: false,
        schedule,
        kind: AppKind::App,
        cpu_pct: 0.0,
        memory_bytes: 0,
        processes: Vec::new(),
    };
    let scheduled = |foreground, cores, priority| {
        Some(AppSchedule {
            state: ScheduleState::Scheduled,
            foreground,
            cores,
            priority,
            adjustment: match (cores, priority) {
                (Some(CoreUse::Efficiency), _) => Adjustment::Yielded,
                (_, PriorityLevel::Higher | PriorityLevel::High) => Adjustment::Raised,
                _ => Adjustment::Protected,
            },
        })
    };
    let other = |state| {
        Some(AppSchedule {
            state,
            foreground: false,
            cores: None,
            priority: PriorityLevel::Normal,
            adjustment: Adjustment::None,
        })
    };
    vec![
        app(
            "netdisk",
            AppPreference::Background,
            scheduled(false, Some(CoreUse::Efficiency), PriorityLevel::Lower),
        ),
        AppEntry {
            cpu_pct: 6.5,
            memory_bytes: 3 * GB,
            processes: vec![
                AppProcess {
                    pid: 100,
                    name: "browser.exe".into(),
                    path: r"c:\apps\browser.exe".into(),
                    preference: None,
                    cpu_pct: 4.0,
                    memory_bytes: 2 * GB,
                    schedule: scheduled(true, Some(CoreUse::Performance), PriorityLevel::Higher)
                        .unwrap(),
                },
                AppProcess {
                    pid: 101,
                    name: "gpu-helper.exe".into(),
                    path: r"c:\apps\helper\gpu-helper.exe".into(),
                    preference: Some(AppPreference::Game),
                    cpu_pct: 2.5,
                    memory_bytes: GB,
                    schedule: other(ScheduleState::SelfManaged).unwrap(),
                },
            ],
            ..app(
                "browser",
                AppPreference::Auto,
                scheduled(true, Some(CoreUse::Performance), PriorityLevel::Higher),
            )
        },
        app(
            "render",
            AppPreference::AlwaysFast,
            scheduled(false, None, PriorityLevel::High),
        ),
        AppEntry {
            kind: AppKind::Excluded,
            ..app("ime", AppPreference::Auto, other(ScheduleState::Excluded))
        },
        app(
            "tool",
            AppPreference::Auto,
            other(ScheduleState::SelfManaged),
        ),
        app("editor", AppPreference::Auto, other(ScheduleState::Off)),
        app("stopped", AppPreference::Game, None),
    ]
}

fn cleanup_overview() -> CleanupOverview {
    CleanupOverview {
        freeable_bytes: 1065 * 1024 * 1024,
        needs_attention: 3,
        mem_used_bytes: 20 * GB,
        mem_total_bytes: 32 * GB,
        restart_advised: true,
        auto_clean: true,
        last_cleanup: None,
        since_boot: Some(SinceBoot {
            baseline_uptime_ms: 5 * 60_000,
            baseline_at_unix_ms: 1_793_000_000_000,
            uptime_ms: 3 * 24 * 3_600_000,
            items: vec![
                TotalChange {
                    item: TotalItem::Handles,
                    at_boot: 82_000,
                    now: 168_000,
                    notable: true,
                },
                TotalChange {
                    item: TotalItem::KernelPool,
                    at_boot: 700 * 1024 * 1024,
                    now: 900 * 1024 * 1024,
                    notable: false,
                },
                TotalChange {
                    item: TotalItem::SystemCache,
                    at_boot: GB,
                    now: 6 * GB,
                    notable: false,
                },
            ],
        }),
        processes: vec![
            ProcessGroup {
                pid: 1,
                name: "Windows 音频设备图形隔离".into(),
                path: Some(r"c:\windows\system32\audiodg.exe".into()),
                process_count: 1,
                memory_bytes: 1065 * 1024 * 1024,
                cpu_pct: 3.1,
                issue: Issue::Oversized,
                growth_per_hour: None,
                since_start: None,
                remedy: Remedy::RestartService {
                    service: "Audiosrv".into(),
                },
                disruptive: true,
            },
            ProcessGroup {
                pid: 20,
                name: "微信".into(),
                path: None,
                process_count: 12,
                memory_bytes: 2 * GB,
                cpu_pct: 0.4,
                issue: Issue::Growing,
                growth_per_hour: Some(80 * 1024 * 1024),
                since_start: None,
                remedy: Remedy::ReopenManually,
                disruptive: false,
            },
            ProcessGroup {
                pid: 30,
                name: "Microsoft Edge".into(),
                path: None,
                process_count: 23,
                memory_bytes: 3 * GB,
                cpu_pct: 1.2,
                issue: Issue::None,
                growth_per_hour: None,
                since_start: None,
                remedy: Remedy::None,
                disruptive: false,
            },
            ProcessGroup {
                pid: 40,
                name: "Service Host: Windows Push Notifications".into(),
                path: Some(r"c:\windows\system32\svchost.exe".into()),
                process_count: 1,
                memory_bytes: 90 * 1024 * 1024,
                cpu_pct: 0.0,
                issue: Issue::Accumulated,
                growth_per_hour: None,
                since_start: Some(Change {
                    metric: GrowthMetric::Handles,
                    from: 2_100,
                    to: 46_000,
                }),
                remedy: Remedy::None,
                disruptive: false,
            },
        ],
    }
}

/// 一个大核域（有程序）和一个小核组（只读得到部分负载）。
fn cores() -> CoreMap {
    let program = |pid, name: &str, verdict, load_pct, domains| PlacedProgram {
        pid,
        name: name.into(),
        path: format!(r"c:\games\{pid}.exe"),
        verdict,
        load_pct,
        domains,
    };
    CoreMap {
        partitioned: true,
        smart_schedule: true,
        demand: Pressure::High,
        domains: vec![
            CoreDomainDto {
                performance: true,
                cores: 6,
                threads: 12,
                l2_bytes: 1280 * 1024,
                l2_threads: 2,
                l3_bytes: 24 << 20,
                l3_group: Some(0),
                load_pct: Some(62.5),
                processors: vec![
                    ProcessorDto {
                        number: 0,
                        core: 0,
                        load_pct: Some(80.0),
                    },
                    ProcessorDto {
                        number: 1,
                        core: 0,
                        load_pct: Some(45.0),
                    },
                ],
                programs: vec![program(7, "League of Legends", Verdict::Playing, 260, 2)],
            },
            CoreDomainDto {
                performance: false,
                cores: 4,
                threads: 4,
                l2_bytes: 2 << 20,
                l2_threads: 4,
                l3_bytes: 24 << 20,
                l3_group: Some(0),
                load_pct: None,
                processors: vec![ProcessorDto {
                    number: 64,
                    core: 256,
                    load_pct: None,
                }],
                programs: vec![
                    program(7, "League of Legends", Verdict::Playing, 260, 2),
                    program(9, "OneDrive", Verdict::Yielded, 35, 1),
                    program(11, "微信", Verdict::Aside, 0, 1),
                ],
            },
        ],
        unrestricted: vec![program(12, "Microsoft Edge", Verdict::Lowered, 18, 2)],
    }
}

/// 每种事件各一条，界面必须都能读懂。
fn events() -> EventPage {
    let program = ProgramRef {
        pid: 7,
        name: "League of Legends".into(),
        path: r"c:\riot games\league of legends.exe".into(),
    };
    let list = vec![
        Event::Demand {
            level: Pressure::High,
            reason: Some(DemandReason::Focus),
        },
        Event::Program {
            program: program.clone(),
            from: Verdict::Normal,
            to: Verdict::Playing,
            cores: CoreSpan {
                all: false,
                main: true,
                others: 1,
            },
            load_pct: 260,
            disk_mb_s: 3,
            cause: None,
        },
        Event::Program {
            program: program.clone(),
            from: Verdict::Yielded,
            to: Verdict::Normal,
            cores: CoreSpan {
                all: true,
                main: true,
                others: 2,
            },
            load_pct: 0,
            disk_mb_s: 0,
            cause: Some(Cause::Foreground),
        },
        Event::MovedAside { count: 12 },
        Event::AccessDenied { program },
        Event::Recovered {
            left: 5,
            restored: 3,
        },
        Event::ModeChanged {
            mode: Mode::Vendor,
            on_battery: false,
        },
        Event::ModeFailed { mode: Mode::Quiet },
        Event::Throttled {
            limit_pct: Some(78.0),
        },
        Event::Unthrottled,
        Event::PlanSwitched,
        Event::SettingRestored {
            setting: Some(PowerSettingKind::MinState),
            class: Some(1),
            ac: true,
            found: Some(100),
            expected: Some(5),
        },
        Event::OptimizationReapplied {
            id: "allow_boost".into(),
        },
        Event::EfficiencySlider,
        Event::OptimizationsApplied {
            ids: vec!["fast_startup".into(), "open_this_pc".into()],
        },
        Event::OptimizationsReverted {
            ids: vec!["fast_startup".into()],
        },
        Event::Cleaned {
            trigger: CleanupTrigger::Auto,
            freed_bytes: GB,
            handled: 2,
            failed: 1,
        },
        Event::Started {
            version: "0.7.0".into(),
            observe_only: true,
        },
        Event::Resumed,
        Event::Paused,
        Event::PreferenceSet {
            name: "OneDrive".into(),
            path: r"c:\program files\microsoft onedrive\onedrive.exe".into(),
            preference: AppPreference::Background,
        },
        Event::SmartSchedule { enabled: false },
        Event::Program {
            program: ProgramRef {
                pid: 9,
                name: "Game".into(),
                path: String::new(),
            },
            from: Verdict::Normal,
            to: Verdict::Focused,
            cores: CoreSpan {
                all: false,
                main: true,
                others: 0,
            },
            load_pct: 35,
            disk_mb_s: 0,
            cause: Some(Cause::LooksLikeGame),
        },
    ];
    EventPage {
        entries: list
            .into_iter()
            .enumerate()
            .map(|(i, event)| EventEntry {
                id: i as u64 + 1,
                at_unix_ms: 1_800_000_000_000 + i as u64 * 1000,
                category: event.category(),
                event,
            })
            .collect(),
        log_dir: r"C:\ProgramData\Poise".into(),
    }
}

/// 一段游戏（两个重点程序、温度读得到）和一段编译（温度读不到）。
fn records() -> Vec<SessionRecord> {
    vec![
        SessionRecord {
            started_unix_ms: 1_800_000_000_000,
            ended_unix_ms: 1_800_002_400_000,
            programs: vec![
                RecordProgram {
                    name: "League of Legends".into(),
                    path: Some(r"c:\riot games\league of legends.exe".into()),
                    game: true,
                    focused_secs: 2350,
                    peak_load_pct: 310,
                    widest_domains: 0b011,
                },
                RecordProgram {
                    name: "Riot Client".into(),
                    path: None,
                    game: false,
                    focused_secs: 40,
                    peak_load_pct: 120,
                    widest_domains: 0b001,
                },
            ],
            modes: vec![Mode::Vendor, Mode::Full],
            moved_aside_peak: 23,
            yielded_programs: 4,
            cpu_pct_avg: 41.5,
            cpu_pct_peak: 88.0,
            cpu_temp_avg: Some(86.5),
            cpu_temp_peak: Some(99.0),
            throttled_secs: 180,
        },
        SessionRecord {
            started_unix_ms: 1_799_990_000_000,
            ended_unix_ms: 1_799_990_300_000,
            programs: vec![],
            modes: vec![Mode::Balanced],
            moved_aside_peak: 0,
            yielded_programs: 0,
            cpu_pct_avg: 70.0,
            cpu_pct_peak: 100.0,
            cpu_temp_avg: None,
            cpu_temp_peak: None,
            throttled_secs: 0,
        },
    ]
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// 与磁盘上的样本比较；设置了 POISE_UPDATE_FIXTURES 时改为写入。
fn check<T: Serialize>(name: &str, value: &T) {
    let path = fixtures_dir().join(name);
    let json = serde_json::to_string_pretty(value).unwrap() + "\n";
    if std::env::var_os("POISE_UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(fixtures_dir()).unwrap();
        std::fs::write(&path, json).unwrap();
        return;
    }
    let stored = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("缺少样本 {name}，用 POISE_UPDATE_FIXTURES=1 生成"));
    assert_eq!(
        stored.replace("\r\n", "\n"),
        json,
        "{name} 与当前数据结构不一致：确认界面兼容后用 POISE_UPDATE_FIXTURES=1 重新生成"
    );
}

#[test]
fn status_full() {
    check("status_full.json", &full_status());
}

#[test]
fn status_minimal() {
    check("status_minimal.json", &minimal_status());
}

#[test]
fn cleanup_result_sample() {
    check("cleanup_result.json", &cleanup_result());
}

#[test]
fn diagnosis_sample() {
    check("diagnosis.json", &diagnosis());
}

#[test]
fn trend_sample() {
    check("trend.json", &trend());
}

#[test]
fn apps_sample() {
    check("apps.json", &apps());
}

#[test]
fn cleanup_overview_sample() {
    check("cleanup_overview.json", &cleanup_overview());
}

#[test]
fn error_response_sample() {
    check(
        "error_response.json",
        &ResponseEnvelope::error(7, ErrorCode::ObserveOnly, "dry run"),
    );
}

#[test]
fn cores_sample() {
    check("cores.json", &cores());
}

#[test]
fn events_sample() {
    check("events.json", &events());
}

#[test]
fn records_sample() {
    check("records.json", &records());
}
