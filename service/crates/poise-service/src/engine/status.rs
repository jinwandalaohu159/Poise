//! 给界面的数据：状态、程序列表、卡顿诊断。

use std::collections::{BTreeMap, HashMap};

use poise_core::classify::Role;
use poise_core::diagnose::{
    self, DiagnoseConfig, Health, HealthCode, Level, StutterInput, StutterReport,
};
use poise_core::model::{Pid, ProcessSample};
use poise_core::prefs::AppPreference;
use poise_hardware::HardwareReading;
use poise_hardware::storage::DiskHealth;
use poise_platform::observe::system;
use poise_protocol::{
    AppEntry, DiskDto, DiskHealthDto, DriveSpace, FanDto, HardwareInfo, MemoryDto, Metrics, Status,
};

use super::Engine;

impl Engine {
    pub fn status(&self) -> Status {
        let view = self.view.as_ref();
        let sys = view.map(|v| v.snapshot.system).unwrap_or_default();
        let default_reading = HardwareReading::default();
        let reading = view.map_or(&default_reading, |v| &v.reading);
        Status {
            health: view.map(|v| v.health.clone()).unwrap_or(Health {
                code: HealthCode::Smooth,
                level: Level::Good,
                game: None,
                cleanable: 0,
                freeable_bytes: 0,
            }),
            mode: self.settings.mode,
            effective_mode: view.map_or(self.settings.mode, |v| v.effective_mode),
            auto_full_for_games: self.settings.auto_full_for_games,
            auto_clean: self.settings.auto_clean,
            metrics: Metrics {
                cpu_pct: self.activity.system_cpu_pct(),
                cpu_temp: reading.thermals.cpu_temp,
                gpu_temp: reading.thermals.gpu_temp,
                gpu_pct: reading.gpu_pct,
                mem_used_pct: sys.mem_used_pct(),
                mem_used_bytes: sys.mem_total.saturating_sub(sys.mem_available),
                mem_total_bytes: sys.mem_total,
                cpu_mhz: sys.cpu_mhz_current,
                on_battery: system::on_battery(),
            },
            hardware: self.hardware_info(reading),
            adjusted_processes: self.scheduler.adjusted_count(),
            deferred_cleanup: self.deferred_count(),
            dry_run: self.cfg.dry_run,
            last_cleanup: self.last_cleanup(),
        }
    }

    fn hardware_info(&self, reading: &HardwareReading) -> HardwareInfo {
        HardwareInfo {
            profile: self.hardware.profile_name().to_string(),
            thermal_control: self.hardware.thermal_control(),
            fans: reading
                .fans
                .iter()
                .map(|f| FanDto {
                    rpm: f.rpm,
                    max_rpm: f.max_rpm,
                })
                .collect(),
            disks: reading
                .storage
                .disks
                .iter()
                .map(|d| DiskDto {
                    name: d.name.clone(),
                    temp_c: d.temp_c,
                    health: match d.health {
                        DiskHealth::Healthy => DiskHealthDto::Healthy,
                        DiskHealth::Warning => DiskHealthDto::Warning,
                        DiskHealth::Unhealthy => DiskHealthDto::Unhealthy,
                        DiskHealth::Unknown => DiskHealthDto::Unknown,
                    },
                    wear_pct: d.wear_pct,
                })
                .collect(),
            system_drive: reading
                .storage
                .system_drive
                .map(|(free_bytes, total_bytes)| DriveSpace {
                    free_bytes,
                    total_bytes,
                }),
            memory: self.hardware.memory().map(|m| MemoryDto {
                total_bytes: m.total_bytes,
                speed_mhz: m.speed_mhz,
                modules: m.modules,
            }),
        }
    }

    /// 程序列表：正在运行、有窗口或在干活的用户程序，加上设置过偏好但没在运行的程序。
    pub fn apps(&self) -> Vec<AppEntry> {
        let mut by_path: BTreeMap<String, AppEntry> = BTreeMap::new();
        let windows_dir = windows_dir();
        if let Some(view) = &self.view {
            let by_pid: HashMap<Pid, &ProcessSample> = view
                .snapshot
                .processes
                .iter()
                .map(|p| (p.facts.key.pid, p))
                .collect();
            // Windows 自带的宿主进程（应用框架宿主、COM 代理等）对用户没有意义。
            let listable = |p: &ProcessSample| {
                view.classes
                    .get(&p.facts.key.pid)
                    .is_some_and(|c| c.role == Role::UserApp)
                    && p.facts
                        .image_path
                        .as_deref()
                        .is_some_and(|path| !is_under(path, &windows_dir))
            };
            for p in &view.snapshot.processes {
                let Some(c) = view.classes.get(&p.facts.key.pid) else {
                    continue;
                };
                if !(listable(p) && (c.has_visible_window || c.recently_active)) {
                    continue;
                }
                // 辅助进程归到所在进程树的主程序名下（偏好也跟随主程序）。
                let app = by_pid
                    .get(&c.root_pid)
                    .copied()
                    .filter(|root| listable(root))
                    .unwrap_or(p);
                let Some(path) = &app.facts.image_path else {
                    continue;
                };
                let entry = by_path.entry(path.clone()).or_insert_with(|| AppEntry {
                    pid: Some(app.facts.key.pid),
                    name: app.facts.friendly_name(),
                    path: path.clone(),
                    preference: self.settings.preferences.lookup(&app.facts),
                    running: true,
                    is_game: false,
                });
                entry.is_game |= c.is_game;
            }
        }
        for (path, pref) in &self.settings.preferences.apps {
            if *pref == AppPreference::Auto || by_path.contains_key(path) {
                continue;
            }
            let name = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            by_path.insert(
                path.clone(),
                AppEntry {
                    pid: None,
                    name,
                    path: path.clone(),
                    preference: *pref,
                    running: false,
                    is_game: *pref == AppPreference::Game,
                },
            );
        }
        let mut apps: Vec<AppEntry> = by_path.into_values().collect();
        apps.sort_by(|a, b| {
            b.running
                .cmp(&a.running)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        apps
    }

    pub fn diagnose(&self) -> Option<StutterReport> {
        let view = self.view.as_ref()?;
        Some(diagnose::analyze_stutter(
            &DiagnoseConfig::default(),
            &StutterInput {
                snapshot: &view.snapshot,
                classes: &view.classes,
                activity: &self.activity,
                thermals: view.reading.thermals,
                suspects: &self.cleanup.suspects,
            },
        ))
    }
}

/// 小写的 Windows 目录，带结尾的反斜杠（与快照里的小写路径比较）。
fn windows_dir() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!("{}\\", root.trim_end_matches('\\').to_lowercase())
}

fn is_under(path: &str, dir: &str) -> bool {
    path.starts_with(dir)
}
