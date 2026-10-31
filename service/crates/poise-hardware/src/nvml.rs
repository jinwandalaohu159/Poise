//! 通用层：通过 NVIDIA 官方的 NVML 读取显卡温度和占用（只读）。
//!
//! 在 Optimus 笔记本上，访问 NVML 会唤醒休眠中的独显，所以由调用方决定何时读取，
//! 并在空闲一段时间后释放库，让独显重新休眠。
//!
//! 开机时服务可能比显卡驱动先就绪：打开失败后隔一会儿再试（间隔逐次加倍），没有驱动文件才不再尝试。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use libloading::{Library, Symbol};
use poise_platform::persist::paths;

type Device = *mut std::ffi::c_void;
type TemperatureFn = unsafe extern "C" fn(Device, u32, *mut u32) -> i32;
type UtilizationFn = unsafe extern "C" fn(Device, *mut Utilization) -> i32;

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
    /// 打开时查好的函数（只在 `lib` 有效期间使用）。
    temperature: Option<TemperatureFn>,
    utilization: Option<UtilizationFn>,
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

/// 打开失败后的重试安排。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// 什么时候再试，以及下次失败后等多久。
    At(Instant, Duration),
    /// 没有驱动文件：不再尝试。
    Never,
}

#[derive(Default)]
pub struct Nvml {
    session: Option<Session>,
    retry: Option<Retry>,
}

impl Nvml {
    const IDLE_RELEASE: Duration = Duration::from_secs(30);
    /// 打开失败后第一次重试的间隔，之后逐次加倍，最长 [`Self::RETRY_MAX`]。
    const RETRY_FIRST: Duration = Duration::from_secs(30);
    const RETRY_MAX: Duration = Duration::from_secs(30 * 60);

    /// 读取一次。没有 NVIDIA 显卡或驱动时返回空读数。
    pub fn read(&mut self) -> GpuReading {
        if self.session.is_none() {
            let now = Instant::now();
            match self.retry {
                Some(Retry::Never) => return GpuReading::default(),
                Some(Retry::At(at, _)) if now < at => return GpuReading::default(),
                _ => {}
            }
            match open() {
                Opened::Ready(s) => {
                    self.session = Some(s);
                    self.retry = None;
                }
                Opened::NoDriver => {
                    self.retry = Some(Retry::Never);
                    return GpuReading::default();
                }
                Opened::Failed => {
                    let wait = match self.retry {
                        Some(Retry::At(_, wait)) => wait,
                        _ => Self::RETRY_FIRST,
                    };
                    self.retry = Some(Retry::At(now + wait, (wait * 2).min(Self::RETRY_MAX)));
                    return GpuReading::default();
                }
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
    paths::windows_dir().join("System32").join("nvml.dll")
}

enum Opened {
    Ready(Session),
    /// 没有驱动文件（不是 NVIDIA 显卡）。
    NoDriver,
    /// 有驱动但这次没打开（驱动可能还没就绪）。
    Failed,
}

fn open() -> Opened {
    let path = nvml_path();
    if !path.exists() {
        return Opened::NoDriver;
    }
    open_session(&path).map_or(Opened::Failed, Opened::Ready)
}

fn open_session(path: &std::path::Path) -> Option<Session> {
    // SAFETY: 加载 NVIDIA 驱动自带的系统 DLL；函数签名来自 NVML 官方头文件。
    unsafe {
        let lib = Library::new(path).ok()?;
        let init: Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"nvmlInit_v2\0").ok()?;
        if init() != NVML_SUCCESS {
            return None;
        }
        let by_index: Symbol<unsafe extern "C" fn(u32, *mut Device) -> i32> =
            lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
        let mut device: Device = std::ptr::null_mut();
        if by_index(0, &mut device) != NVML_SUCCESS {
            // 已经初始化：由 Session 的 Drop 配对关闭。
            drop(Session {
                lib,
                device,
                temperature: None,
                utilization: None,
                last_used: Instant::now(),
            });
            return None;
        }
        let temperature = lib
            .get::<TemperatureFn>(b"nvmlDeviceGetTemperature\0")
            .ok()
            .map(|f| *f);
        let utilization = lib
            .get::<UtilizationFn>(b"nvmlDeviceGetUtilizationRates\0")
            .ok()
            .map(|f| *f);
        Some(Session {
            lib,
            device,
            temperature,
            utilization,
            last_used: Instant::now(),
        })
    }
}

fn read_device(s: &Session) -> GpuReading {
    let mut out = GpuReading::default();
    // SAFETY: 函数与设备句柄来自同一个已初始化、仍然加载着的 NVML 会话。
    unsafe {
        if let Some(f) = s.temperature {
            let mut t = 0u32;
            if f(s.device, NVML_TEMPERATURE_GPU, &mut t) == NVML_SUCCESS {
                out.temp = Some(t as f32);
            }
        }
        if let Some(f) = s.utilization {
            let mut u = Utilization::default();
            if f(s.device, &mut u) == NVML_SUCCESS {
                out.utilization = Some(u.gpu as f32);
            }
        }
    }
    out
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
