//! 戴尔 AWCC BIOS 接口（WMI 类 AWCCWmiMethodFunction）：温度、风扇转速、散热模式。
//!
//! 只使用 BIOS 自己报告的 ID：先读"有几个风扇、几个传感器、几种模式"，再按序号读出各自的 ID。
//! 写入只有三种：切换散热模式（目标必须在 BIOS 报告的模式列表里，G 模式除外）、风扇加速（风扇必须在
//! BIOS 报告的风扇列表里，强度 0～255，0 即不加速）、G 模式开关。

use poise_platform::win::wmi::{WmiError, WmiNamespace, WmiResult};

use super::FanReading;

const NAMESPACE: &str = r"root\WMI";
const CLASS: &str = "AWCCWmiMethodFunction";
const READ: &str = "Thermal_Information";
const CONTROL: &str = "Thermal_Control";
/// G 模式开关（G 系列机型）。功能号：0x01 切换，0x02 读取（1 = 开）。
const GAME_SHIFT: &str = "GameShiftStatus";
const GAME_SHIFT_TOGGLE: u8 = 0x01;
const GAME_SHIFT_READ: u8 = 0x02;
/// G 模式对应的散热模式。BIOS 的模式列表里没有它，需要先打开 G 模式开关。
/// G15 5520 实测：打开后散热模式读数为 0xAB，风扇 7 秒内升到约 4600 RPM；关闭后恢复原状。
pub const G_MODE: u8 = 0xAB;

/// Thermal_Information 的功能号（参数低字节），高字节是功能的参数。
mod info {
    /// 返回数量：字节 0 风扇数，字节 1 传感器数，字节 3 散热模式数。
    pub const DESCRIBE: u8 = 0x02;
    /// 按序号返回 ID：先是风扇，再是传感器，最后是散热模式。
    pub const ID_AT: u8 = 0x03;
    pub const TEMPERATURE: u8 = 0x04;
    pub const FAN_RPM: u8 = 0x05;
    pub const FAN_MAX_RPM: u8 = 0x09;
    pub const POWER_MODE: u8 = 0x0B;
    /// 风扇当前的加速值（0～255）。
    pub const FAN_BOOST: u8 = 0x0C;
}
/// Thermal_Control 的功能号。
const CONTROL_POWER_MODE: u8 = 0x01;
/// 风扇加速：参数为风扇 ID 与加速值（0～255）。G15 5520 实测：均衡模式下 128 → 约 3500 RPM，
/// 255 → 约 4500 RPM（最高 4800），设回 0 后转速随温度回落。
const CONTROL_FAN_BOOST: u8 = 0x02;
/// BIOS 表示"失败 / 不支持"的返回值。
const BIOS_ERROR: u32 = 0xFFFF_FFFF;

fn arg(function: u8, value: u8) -> u32 {
    args(function, value, 0)
}

/// 参数按字节排列：功能号、参数 1、参数 2。
fn args(function: u8, first: u8, second: u8) -> u32 {
    u32::from(function) | (u32::from(first) << 8) | (u32::from(second) << 16)
}

/// BIOS 报告的设备布局。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub fans: Vec<u8>,
    pub sensors: Vec<u8>,
    pub power_modes: Vec<u8>,
}

impl Layout {
    /// 由"数量描述"和按序号读出的 ID 组装布局。`id_at` 读失败时返回 None。
    pub fn read(description: u32, mut id_at: impl FnMut(u8) -> Option<u32>) -> Option<Self> {
        let [fans, sensors, _, modes] = description.to_le_bytes();
        let mut index = 0u8;
        let mut take = |count: u8| -> Option<Vec<u8>> {
            (0..count)
                .map(|_| {
                    let id = id_at(index)?;
                    index += 1;
                    // 传感器的 ID 带有类别标记（例如 0x101），只用低字节。
                    Some((id & 0xFF) as u8)
                })
                .collect()
        };
        Some(Self {
            fans: take(fans)?,
            sensors: take(sensors)?,
            power_modes: take(modes)?,
        })
    }
}

pub struct Awcc {
    ns: WmiNamespace,
    path: String,
    layout: Layout,
}

impl Awcc {
    /// 连接 BIOS 接口并读出布局。不是戴尔机器、或者没有权限时返回错误。
    pub fn open() -> WmiResult<Self> {
        let ns = WmiNamespace::connect(NAMESPACE)?;
        let path = ns
            .query(&format!("SELECT __PATH FROM {CLASS}"))?
            .into_iter()
            .find_map(|o| o.string("__PATH"))
            .ok_or_else(|| WmiError("没有 AWCC 接口".into()))?;
        let mut awcc = Self {
            ns,
            path,
            layout: Layout::default(),
        };
        let description = awcc.read(info::DESCRIBE, 0)?;
        awcc.layout = Layout::read(description, |i| awcc.read(info::ID_AT, i).ok())
            .ok_or_else(|| WmiError("读取风扇和传感器列表失败".into()))?;
        Ok(awcc)
    }

    /// 同一接口的另一条连接（沿用已读出的布局），给采集线程只读使用。
    pub fn reconnect(&self) -> WmiResult<Self> {
        Ok(Self {
            ns: WmiNamespace::connect(NAMESPACE)?,
            path: self.path.clone(),
            layout: self.layout.clone(),
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn call(&self, method: &str, function: u8, value: u8) -> WmiResult<u32> {
        self.call_raw(method, arg(function, value))
    }

    fn call_raw(&self, method: &str, argument: u32) -> WmiResult<u32> {
        let r = self
            .ns
            .call_u32(CLASS, &self.path, method, "arg2", argument, "argr")?;
        if r == BIOS_ERROR {
            Err(WmiError(format!("BIOS 不支持 {method}({argument:#x})")))
        } else {
            Ok(r)
        }
    }

    fn read(&self, function: u8, value: u8) -> WmiResult<u32> {
        self.call(READ, function, value)
    }

    /// 传感器温度（摄氏度）。
    pub fn temperature(&self, sensor: u8) -> Option<f32> {
        self.read(info::TEMPERATURE, sensor)
            .ok()
            .filter(|&t| crate::plausible_celsius(u64::from(t)))
            .map(|t| t as f32)
    }

    pub fn fans(&self) -> Vec<FanReading> {
        self.layout
            .fans
            .iter()
            .filter_map(|&fan| {
                Some(FanReading {
                    rpm: self.read(info::FAN_RPM, fan).ok()?,
                    max_rpm: self.read(info::FAN_MAX_RPM, fan).ok()?,
                })
            })
            .collect()
    }

    pub fn power_mode(&self) -> Option<u8> {
        self.read(info::POWER_MODE, 0).ok().map(|m| m as u8)
    }

    /// 风扇当前的加速值（0～255）。
    pub fn fan_boost(&self, fan: u8) -> Option<u8> {
        self.read(info::FAN_BOOST, fan)
            .ok()
            .and_then(|v| u8::try_from(v).ok())
    }

    /// 设置风扇加速（0 = 不加速，由 BIOS 按温度调节）。只接受 BIOS 自己报告的风扇。
    pub fn set_fan_boost(&self, fan: u8, value: u8) -> Result<(), String> {
        if !self.layout.fans.contains(&fan) {
            return Err(format!("BIOS 没有列出风扇 {fan:#x}"));
        }
        self.call_raw(CONTROL, args(CONTROL_FAN_BOOST, fan, value))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// G 模式开关的状态；机型没有这个接口时为 None。
    pub fn g_mode(&self) -> Option<bool> {
        self.call_raw(GAME_SHIFT, u32::from(GAME_SHIFT_READ))
            .ok()
            .map(|v| v == 1)
    }

    /// 打开或关闭 G 模式：开关状态不同时才切换；打开时同时进入 G 模式的散热模式。
    /// 关闭后由调用方切到目标散热模式。
    pub fn set_g_mode(&self, on: bool) -> Result<(), String> {
        let current = self.g_mode().ok_or("BIOS 没有 G 模式开关")?;
        if current != on {
            self.call_raw(GAME_SHIFT, u32::from(GAME_SHIFT_TOGGLE))
                .map_err(|e| e.to_string())?;
        }
        if on {
            self.call(CONTROL, CONTROL_POWER_MODE, G_MODE)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// 切换散热模式。只接受 BIOS 自己报告的模式。
    pub fn set_power_mode(&self, mode: u8) -> Result<(), String> {
        if !self.layout.power_modes.contains(&mode) {
            return Err(format!("BIOS 没有列出散热模式 {mode:#x}"));
        }
        self.call(CONTROL, CONTROL_POWER_MODE, mode)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// G15 5520 实测：2 个风扇、2 个传感器、4 种散热模式。
    const G15_DESCRIPTION: u32 = 0x0400_0202;
    const G15_IDS: [u32; 8] = [0x32, 0x33, 0x101, 0x106, 0xA0, 0xA1, 0xA5, 0xA3];

    #[test]
    fn decodes_the_g15_layout() {
        let layout = Layout::read(G15_DESCRIPTION, |i| G15_IDS.get(i as usize).copied()).unwrap();
        assert_eq!(layout.fans, [0x32, 0x33]);
        assert_eq!(layout.sensors, [0x01, 0x06]);
        assert_eq!(layout.power_modes, [0xA0, 0xA1, 0xA5, 0xA3]);
    }

    #[test]
    fn failed_id_read_fails_the_layout() {
        assert!(Layout::read(G15_DESCRIPTION, |i| (i < 3).then_some(0x32)).is_none());
    }

    #[test]
    fn packs_function_and_argument() {
        assert_eq!(arg(info::TEMPERATURE, 0x06), 0x0604);
        assert_eq!(arg(CONTROL_POWER_MODE, 0xA3), 0xA301);
        assert_eq!(args(CONTROL_FAN_BOOST, 0x32, 0x80), 0x0080_3202);
    }
}
