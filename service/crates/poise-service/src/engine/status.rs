//! 给界面的数据：状态（含硬件信息）、卡顿诊断、走势。

use poise_core::diagnose::HISTORY_WINDOW_MS;
use poise_core::diagnose::{self, DiagnoseConfig, StutterInput};
use poise_hardware::HardwareReading;
use poise_protocol::{Diagnosis, HardwareInfo, Metrics, Status, Trend};

use super::{Engine, convert};

impl Engine {
    pub fn status(&self) -> Status {
        let view = self.view.as_ref();
        let sys = view.map(|v| v.snapshot.system).unwrap_or_default();
        let default_reading = HardwareReading::default();
        let reading = view.map_or(&default_reading, |v| &v.reading);
        Status {
            health: view.map(|v| v.health.clone()).unwrap_or_default(),
            mode: self.settings.mode,
            effective_mode: view.map_or(self.settings.mode, |v| v.effective_mode),
            game_mode: self.settings.game_mode,
            auto_clean: self.settings.auto_clean,
            fan_boost: self.settings.fan_boost,
            smart_schedule: self.settings.smart_schedule,
            metrics: Metrics {
                cpu_pct: self.activity.system_cpu_pct(),
                cpu_temp: reading.thermals.cpu_temp,
                gpu_temp: reading.thermals.gpu_temp,
                gpu_pct: reading.gpu_pct,
                mem_used_pct: sys.mem_used_pct(),
                mem_used_bytes: sys.mem_used_bytes(),
                mem_total_bytes: sys.mem_total,
                cpu_mhz: sys.cpu_mhz_current,
                on_battery: sys.on_battery,
            },
            hardware: self.hardware_info(reading),
            adjusted_processes: self.scheduler.adjusted_count(),
            deferred_cleanup: self.deferred_count(),
            dry_run: self.cfg.dry_run,
            paused: self.paused,
            last_cleanup: self.last_cleanup(),
        }
    }

    fn hardware_info(&self, reading: &HardwareReading) -> HardwareInfo {
        HardwareInfo {
            profile: self.hardware.profile_name().to_string(),
            thermal_control: self.hardware.thermal_control(),
            fan_boost_supported: self.hardware.fan_boost_supported(),
            vendor_mode: self.hardware.vendor_mode().map(String::from),
            fans: reading.fans.iter().map(convert::fan).collect(),
            disks: reading.storage.disks.iter().map(convert::disk).collect(),
            system_drive: reading.storage.system_drive.map(convert::drive_space),
            memory: self.hardware.memory().map(convert::memory),
            cpu: convert::cpu(&self.topology),
        }
    }

    pub fn diagnose(&self) -> Option<Diagnosis> {
        let view = self.view.as_ref()?;
        let report = diagnose::analyze_stutter(
            &DiagnoseConfig::default(),
            &StutterInput {
                snapshot: &view.snapshot,
                classes: &view.classes,
                activity: &self.activity,
                thermals: view.reading.thermals,
                suspects: self.leak_suspects(),
            },
        );
        Some(Diagnosis {
            report,
            smart_schedule: self.settings.smart_schedule,
        })
    }

    pub fn trend(&self) -> Trend {
        Trend {
            window_secs: (HISTORY_WINDOW_MS / 1000) as u32,
            points: self.history.recent(self.now()),
        }
    }
}
