//! 与界面（C#）之间的契约样本。
//!
//! 这里生成的 JSON 存在 `tests/fixtures/`，界面的测试（Poise.Core.Tests）读取同一批文件反序列化，
//! 两边任何一方改了字段名、枚举值或可空性，都会有一边的测试失败。
//! 修改数据结构后用 `POISE_UPDATE_FIXTURES=1 cargo test -p poise-protocol` 重新生成，再跑界面测试。

use std::path::PathBuf;

use poise_core::diagnose::{Health, HealthCode, Level, StutterCause, StutterReport};
use poise_core::prefs::Mode;
use poise_protocol::{
    AdvisoryDto, CleanupResult, CleanupTrigger, DiskDto, DiskHealthDto, DriveSpace, ErrorCode,
    FanDto, HardwareInfo, LastCleanup, MemoryDto, Metrics, ResponseEnvelope, Status,
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
        auto_full_for_games: true,
        auto_clean: false,
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
                temp_c: Some(45),
                health: DiskHealthDto::Warning,
                wear_pct: Some(3),
            }],
            system_drive: Some(DriveSpace {
                free_bytes: 12 * GB,
                total_bytes: 500 * GB,
            }),
            memory: Some(MemoryDto {
                total_bytes: 32 * GB,
                speed_mhz: 4800,
                modules: 2,
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
        fans: vec![],
        disks: vec![DiskDto {
            name: "Disk".into(),
            temp_c: None,
            health: DiskHealthDto::Unknown,
            wear_pct: None,
        }],
        system_drive: None,
        memory: None,
    };
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
                private_bytes: 2 * GB,
            },
            AdvisoryDto::RestartRecommended,
            AdvisoryDto::ManualPending { count: 2 },
        ],
    }
}

fn stutter_report() -> StutterReport {
    StutterReport {
        cause: StutterCause::BackgroundCpu {
            name: "OneDrive".into(),
            core_pct: 85.0,
        },
        cpu_pct: 40.0,
        mem_used_pct: 70.0,
        top_cpu: vec![("OneDrive".into(), 12.0), ("Chrome".into(), 4.5)],
        top_memory: vec![("Chrome".into(), 3 * GB)],
    }
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
fn stutter_report_sample() {
    check("stutter_report.json", &stutter_report());
}

#[test]
fn error_response_sample() {
    check(
        "error_response.json",
        &ResponseEnvelope::error(7, ErrorCode::ObserveOnly, "dry run"),
    );
}
