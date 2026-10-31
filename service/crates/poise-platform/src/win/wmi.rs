//! 最小的 WMI 客户端：连接命名空间、查询对象、调用方法。
//!
//! 只在本机使用（不连接远程计算机）。COM 以多线程模式初始化：服务的多个线程都可能调用，
//! WMI 代理对象在多线程套间里可以跨线程使用。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE, WbemLocator,
};
use windows::core::BSTR;

use super::com;
use super::wide::WideString;

#[derive(Debug)]
pub struct WmiError(pub String);

impl std::fmt::Display for WmiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<windows::core::Error> for WmiError {
    fn from(e: windows::core::Error) -> Self {
        Self(e.message())
    }
}

pub type WmiResult<T> = Result<T, WmiError>;

/// 一个已连接的 WMI 命名空间。
pub struct WmiNamespace {
    services: IWbemServices,
    /// 调用过的方法的入参定义（按"类.方法"）：每次调用都先取类、再取方法定义，要多跑两趟 WMI 服务。
    signatures: RefCell<HashMap<String, IWbemClassObject>>,
}

// WMI 代理对象在多线程套间中可以跨线程使用；所有调用前都会确保当前线程已加入多线程套间。
unsafe impl Send for WmiNamespace {}

impl WmiNamespace {
    /// 连接本机命名空间，例如 `root\WMI`。
    pub fn connect(namespace: &str) -> WmiResult<Self> {
        com::ensure_initialized();
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
            let services = locator.ConnectServer(
                &BSTR::from(namespace),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )?;
            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?;
            Ok(Self {
                services,
                signatures: RefCell::default(),
            })
        }
    }

    /// 执行 WQL 查询，返回全部结果。
    pub fn query(&self, wql: &str) -> WmiResult<Vec<WmiObject>> {
        com::ensure_initialized();
        let mut out = Vec::new();
        unsafe {
            let rows = self.services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(wql),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )?;
            loop {
                let mut row = [None];
                let mut returned = 0;
                let hr = rows.Next(WBEM_INFINITE, &mut row, &mut returned);
                hr.ok()?;
                match row[0].take() {
                    Some(obj) if returned == 1 => out.push(WmiObject(obj)),
                    _ => break,
                }
            }
        }
        Ok(out)
    }

    /// 调用对象（`path` 为对象的 `__PATH`）上只有一个 uint32 入参的方法，返回名为 `out` 的 uint32 出参。
    pub fn call_u32(
        &self,
        class: &str,
        path: &str,
        method: &str,
        arg_name: &str,
        arg: u32,
        out: &str,
    ) -> WmiResult<u32> {
        com::ensure_initialized();
        unsafe {
            let params = self.signature(class, method)?.SpawnInstance(0)?;
            // WMI 用 VT_I4 表示 uint32。
            let value = VARIANT::from(arg as i32);
            let arg_w = WideString::new(arg_name);
            params.Put(arg_w.as_pcwstr(), 0, &value, 0)?;
            let mut result = None;
            self.services.ExecMethod(
                &BSTR::from(path),
                &BSTR::from(method),
                Default::default(),
                None,
                &params,
                Some(&mut result),
                None,
            )?;
            let result = result.ok_or_else(|| WmiError(format!("{method} 没有返回值")))?;
            WmiObject(result)
                .u64(out)
                .map(|v| v as u32)
                .ok_or_else(|| WmiError(format!("{method} 返回值无法解析")))
        }
    }
}

impl WmiNamespace {
    /// 方法的入参定义（第一次用到时取，之后复用）。
    fn signature(&self, class: &str, method: &str) -> WmiResult<IWbemClassObject> {
        let key = format!("{class}.{method}");
        if let Some(sig) = self.signatures.borrow().get(&key) {
            return Ok(sig.clone());
        }
        // SAFETY: 标准的 WMI 调用，接口对象由 windows crate 管理引用计数。
        let sig = unsafe {
            let mut class_obj = None;
            self.services.GetObject(
                &BSTR::from(class),
                Default::default(),
                None,
                Some(&mut class_obj),
                None,
            )?;
            let class_obj = class_obj.ok_or_else(|| WmiError(format!("没有类 {class}")))?;
            let method_w = WideString::new(method);
            let mut in_sig = None;
            class_obj.GetMethod(method_w.as_pcwstr(), 0, &mut in_sig, std::ptr::null_mut())?;
            in_sig.ok_or_else(|| WmiError(format!("方法 {method} 没有入参")))?
        };
        self.signatures.borrow_mut().insert(key, sig.clone());
        Ok(sig)
    }
}

/// 查询结果中的一个对象。
pub struct WmiObject(IWbemClassObject);

impl WmiObject {
    fn get(&self, name: &str) -> Option<VARIANT> {
        let name_w = WideString::new(name);
        let mut value = VARIANT::default();
        unsafe {
            self.0
                .Get(name_w.as_pcwstr(), 0, &mut value, None, None)
                .ok()?;
        }
        (!value.is_empty()).then_some(value)
    }

    /// 整数属性。WMI 把 64 位整数表示为字符串，这里一并处理。
    pub fn u64(&self, name: &str) -> Option<u64> {
        let v = self.get(name)?;
        if let Ok(i) = i32::try_from(&v) {
            return Some(i as u32 as u64);
        }
        if let Ok(u) = u64::try_from(&v) {
            return Some(u);
        }
        BSTR::try_from(&v).ok()?.to_string().trim().parse().ok()
    }

    pub fn string(&self, name: &str) -> Option<String> {
        let v = self.get(name)?;
        BSTR::try_from(&v).ok().map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_strings_and_64_bit_numbers() {
        let ns = WmiNamespace::connect(r"root\cimv2").unwrap();
        let os = ns
            .query("SELECT Caption, TotalVisibleMemorySize FROM Win32_OperatingSystem")
            .unwrap();
        assert_eq!(os.len(), 1);
        assert!(os[0].string("Caption").unwrap().contains("Windows"));
        // uint64 属性在 WMI 里是字符串。
        assert!(os[0].u64("TotalVisibleMemorySize").unwrap() > 0);
        assert!(os[0].u64("NoSuchProperty").is_none());
    }
}
