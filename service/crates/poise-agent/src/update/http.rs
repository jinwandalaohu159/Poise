//! 最小的 HTTPS GET（WinHTTP，系统自带，跟随系统代理设置）：只用于查询 GitHub 上的最新版本。

use std::ffi::c_void;

use poise_platform::win::wide::WideString;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

/// 解析、连接、发送、接收各阶段的超时（毫秒）。
const TIMEOUT_MS: i32 = 15_000;
/// 响应体的上限：版本信息只有几 KB，超过说明不是预期的内容。
const MAX_BODY: usize = 1 << 20;

/// WinHTTP 句柄，离开作用域时关闭。
struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void, what: &str) -> Result<Self, String> {
        if raw.is_null() {
            Err(format!(
                "{what}失败：{}",
                windows::core::Error::from_thread()
            ))
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本对象独占，只关闭一次。
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// 请求 `https://{host}{path}`，返回状态码和响应体。`headers` 每行一个，以 "\r\n" 分隔。
pub fn get(
    host: &str,
    path: &str,
    user_agent: &str,
    headers: &str,
) -> Result<(u32, Vec<u8>), String> {
    let agent = WideString::new(user_agent);
    let host = WideString::new(host);
    let path = WideString::new(path);
    let extra: Vec<u16> = headers.encode_utf16().collect();
    // SAFETY: 所有字符串在调用期间有效；句柄由 Handle 管理，按子到父的顺序（声明的逆序）关闭。
    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                agent.as_pcwstr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "打开会话",
        )?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
            .map_err(|e| format!("设置超时失败：{e}"))?;
        let connection = Handle::new(
            WinHttpConnect(session.0, host.as_pcwstr(), INTERNET_DEFAULT_HTTPS_PORT, 0),
            "连接",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                path.as_pcwstr(),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "创建请求",
        )?;
        if !extra.is_empty() {
            WinHttpAddRequestHeaders(request.0, &extra, WINHTTP_ADDREQ_FLAG_ADD)
                .map_err(|e| format!("添加请求头失败：{e}"))?;
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|e| format!("发送失败：{e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("接收失败：{e}"))?;
        let status = status_code(&request)?;
        Ok((status, read_body(&request)?))
    }
}

fn status_code(request: &Handle) -> Result<u32, String> {
    let mut status = 0u32;
    let mut len = size_of::<u32>() as u32;
    // SAFETY: 缓冲区是本地 u32，长度与之一致。
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
    }
    .map_err(|e| format!("读取状态码失败：{e}"))?;
    Ok(status)
}

fn read_body(request: &Handle) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut available = 0u32;
        // SAFETY: 输出到本地变量。
        unsafe { WinHttpQueryDataAvailable(request.0, &mut available) }
            .map_err(|e| format!("读取失败：{e}"))?;
        if available == 0 {
            return Ok(body);
        }
        if body.len() + available as usize > MAX_BODY {
            return Err("响应过大".into());
        }
        let start = body.len();
        body.resize(start + available as usize, 0);
        let mut read = 0u32;
        // SAFETY: 缓冲区刚扩展出 available 字节。
        unsafe {
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr() as *mut c_void,
                available,
                &mut read,
            )
        }
        .map_err(|e| format!("读取失败：{e}"))?;
        body.truncate(start + read as usize);
    }
}
