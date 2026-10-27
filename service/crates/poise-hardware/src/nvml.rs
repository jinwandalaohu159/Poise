//! 通用层：通过 NVIDIA 官方的 NVML 读取显卡温度和占用（只读）。
//!
//! 在 Optimus 笔记本上，访问 NVML 会唤醒休眠中的独显，所以由调用方决定何时读取，
//! 并在空闲一段时间后释放库，让独显重新休眠。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use libloading::{Library, Symbol};

type Device = *mut std::ffi::c_void;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

const NVML_SUCCESS: i32 = 0;
const NVML_TEMPERATURE_GPU: u32 = 0;

struct Session {
    lib: Library,
    device: Device,
    last_used: Instant,
}

// SAFETY: NVML 官方文档说明其 API 是线程安全的，设备句柄可跨线程使用。
unsafe impl Send for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: 与 nvmlInit_v2 配对调用。
        unsafe {
            if let Ok(shutdown) = self
                .lib
                .get::<unsafe extern "C" fn() -> i32>(b"nvmlShutdown\0")
            {
                shutdown();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuReading {
    pub temp: Option<f32>,
    pub utilization: Option<f32>,
}

#[derive(Default)]
pub struct Nvml {
    session: Option<Session>,
    unavailable: bool,
}

impl Nvml {
    const IDLE_RELEASE: Duration = Duration::from_secs(30);

    /// 读取一次。没有 NVIDIA 显卡或驱动时返回空读数，之后不再尝试。
    pub fn read(&mut self) -> GpuReading {
        if self.unavailable {
            return GpuReading::default();
        }
        if self.session.is_none() {
            self.session = open();
            if self.session.is_none() {
                self.unavailable = true;
                return GpuReading::default();
            }
        }
        let Some(s) = self.session.as_mut() else {
            return GpuReading::default();
        };
        s.last_used = Instant::now();
        read_device(s)
    }

    /// 空闲超过 30 秒就释放，让独显可以重新休眠。
    pub fn release_if_idle(&mut self) {
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.last_used.elapsed() >= Self::IDLE_RELEASE)
        {
            self.session = None;
        }
    }
}

fn nvml_path() -> PathBuf {
    // 使用完整路径加载，避免 DLL 搜索路径劫持。
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("System32").join("nvml.dll")
}

fn open() -> Option<Session> {
    let path = nvml_path();
    if !path.exists() {
        return None;
    }
    // SAFETY: 加载 NVIDIA 驱动自带的系统 DLL；函数签名来自 NVML 官方头文件。
    unsafe {
        let lib = Library::new(&path).ok()?;
        let init: Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"nvmlInit_v2\0").ok()?;
        if init() != NVML_SUCCESS {
            return None;
        }
        let by_index: Symbol<unsafe extern "C" fn(u32, *mut Device) -> i32> =
            lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
        let mut device: Device = std::ptr::null_mut();
        if by_index(0, &mut device) != NVML_SUCCESS {
            return None;
        }
        Some(Session {
            lib,
            device,
            last_used: Instant::now(),
        })
    }
}

fn read_device(s: &Session) -> GpuReading {
    // SAFETY: 设备句柄来自同一个已初始化的 NVML 会话。
    unsafe {
        let mut out = GpuReading::default();
        if let Ok(f) = s
            .lib
            .get::<unsafe extern "C" fn(Device, u32, *mut u32) -> i32>(
                b"nvmlDeviceGetTemperature\0",
            )
        {
            let mut t = 0u32;
            if f(s.device, NVML_TEMPERATURE_GPU, &mut t) == NVML_SUCCESS {
                out.temp = Some(t as f32);
            }
        }
        if let Ok(f) = s
            .lib
            .get::<unsafe extern "C" fn(Device, *mut Utilization) -> i32>(
                b"nvmlDeviceGetUtilizationRates\0",
            )
        {
            let mut u = Utilization::default();
            if f(s.device, &mut u) == NVML_SUCCESS {
                out.utilization = Some(u.gpu as f32);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_gpu_or_reports_unavailable() {
        let mut n = super::Nvml::default();
        let r = n.read();
        if let Some(t) = r.temp {
            assert!((10.0..120.0).contains(&t));
        }
        n.release_if_idle();
    }
}
