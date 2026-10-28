//! 内部数据 → 协议数据（给界面）。只做字段对应，不做判断。

use poise_core::action::{CoreSet, EcoQos};
use poise_core::app_groups::GroupKind;
use poise_core::cleanup::Advisory;
use poise_core::model::PriorityClass;
use poise_core::schedule::Desired;
use poise_hardware::memory::MemoryInfo;
use poise_hardware::profiles::FanReading;
use poise_hardware::storage::{DiskHealth, DiskReading};
use poise_platform::observe::topology::CpuTopology;
use poise_protocol::{
    Adjustment, AdvisoryDto, AppKind, CpuDto, DiskDto, DiskHealthDto, DriveSpace, FanDto,
    MemoryDto, PriorityLevel, VolumeDto,
};

pub(super) fn fan(f: &FanReading) -> FanDto {
    FanDto {
        rpm: f.rpm,
        max_rpm: f.max_rpm,
    }
}

pub(super) fn disk(d: &DiskReading) -> DiskDto {
    DiskDto {
        name: d.name.clone(),
        size_bytes: d.size_bytes,
        temp_c: d.temp_c,
        health: match d.health {
            DiskHealth::Healthy => DiskHealthDto::Healthy,
            DiskHealth::Warning => DiskHealthDto::Warning,
            DiskHealth::Unhealthy => DiskHealthDto::Unhealthy,
            DiskHealth::Unknown => DiskHealthDto::Unknown,
        },
        wear_pct: d.wear_pct,
        volumes: d
            .volumes
            .iter()
            .map(|v| VolumeDto {
                letter: v.letter.to_string(),
                free_bytes: v.free_bytes,
                total_bytes: v.total_bytes,
            })
            .collect(),
    }
}

/// 系统盘（剩余，总大小）。
pub(super) fn drive_space((free_bytes, total_bytes): (u64, u64)) -> DriveSpace {
    DriveSpace {
        free_bytes,
        total_bytes,
    }
}

pub(super) fn memory(m: &MemoryInfo) -> MemoryDto {
    MemoryDto {
        total_bytes: m.total_bytes,
        speed_mhz: m.speed_mhz,
        module_bytes: m.module_bytes.clone(),
    }
}

/// 处理器组成；读不到 CPU 集信息时为空。
pub(super) fn cpu(t: &CpuTopology) -> Option<CpuDto> {
    (t.performance_count.threads > 0).then_some(CpuDto {
        performance_cores: t.performance_count.cores,
        performance_threads: t.performance_count.threads,
        efficiency_cores: t.efficiency_count.cores,
        efficiency_threads: t.efficiency_count.threads,
    })
}

/// 程序分组 → 程序偏好页的分栏。
pub(super) fn app_kind(kind: GroupKind) -> AppKind {
    match kind {
        GroupKind::App => AppKind::App,
        GroupKind::Excluded => AppKind::Excluded,
    }
}

pub(super) fn priority_level(p: PriorityClass) -> PriorityLevel {
    match p {
        PriorityClass::Idle | PriorityClass::BelowNormal => PriorityLevel::Lower,
        PriorityClass::Normal => PriorityLevel::Normal,
        PriorityClass::AboveNormal => PriorityLevel::Higher,
        PriorityClass::High | PriorityClass::Realtime => PriorityLevel::High,
    }
}

/// 已应用的调度状态 → Poise 对它做了什么。
pub(super) fn adjustment(d: Desired) -> Adjustment {
    if d.priority
        .is_some_and(|p| p.rank() > PriorityClass::Normal.rank())
    {
        Adjustment::Raised
    } else if d.priority.is_some() || d.cores == CoreSet::Efficiency || d.eco == EcoQos::On {
        Adjustment::Yielded
    } else if d.eco == EcoQos::Off {
        Adjustment::Protected
    } else {
        Adjustment::None
    }
}

pub(super) fn advisory(a: &Advisory) -> AdvisoryDto {
    match a {
        Advisory::HeavyApp {
            name, memory_bytes, ..
        } => AdvisoryDto::HeavyApp {
            name: name.clone(),
            memory_bytes: *memory_bytes,
        },
        Advisory::RestartRecommended { .. } => AdvisoryDto::RestartRecommended,
        Advisory::ManualPending { count } => AdvisoryDto::ManualPending { count: *count },
    }
}
