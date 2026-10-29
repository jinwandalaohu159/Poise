//! 最小的 HTTPS GET（WinHTTP，系统自带，跟随系统代理设置，自动跟随重定向）：查询最新版本、下载安装程序。

use std::ffi::c_void;

use poise_platform::win::wide::WideString;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

/// 解析、连接、发送的超时（毫秒）。
const CONNECT_TIMEOUT_MS: i32 = 15_000;
/// 等待响应、每次读取的超时（毫秒）：网络慢时下载可以慢，但不能停住不动。
const RECEIVE_TIMEOUT_MS: i32 = 30_000;

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

// WinHTTP 的同步句柄可以在线程之间移动（同一时间只有一个线程使用）。
unsafe impl Send for Handle {}

/// 已收到响应头的请求，响应体按需读取。字段按声明顺序释放：请求、连接、会话。
pub struct Response {
    request: Handle,
    _connection: Handle,
    _session: Handle,
    status: u32,
}

impl Response {
    pub fn status(&self) -> u32 {
        self.status
    }

    /// 读取一段响应体，返回读到的字节数；0 表示读完了。
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let mut read = 0u32;
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: 缓冲区在调用期间有效，长度不超过它的大小。
        unsafe {
            WinHttpReadData(
                self.request.0,
                buf.as_mut_ptr() as *mut c_void,
                len,
                &mut read,
            )
        }
        .map_err(|e| format!("读取失败：{e}"))?;
        Ok(read as usize)
    }

    /// 读完整个响应体；超过 `max` 字节时出错（不是预期的内容）。
    pub fn read_to_end(mut self, max: usize) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                return Ok(body);
            }
            if body.len() + n > max {
                return Err("响应过大".into());
            }
            body.extend_from_slice(&buf[..n]);
        }
    }
}

/// GET `url`（只接受 https），返回收到响应头的请求。`headers` 每行一个，以 "\r\n" 分隔。
pub fn get(url: &str, user_agent: &str, headers: &str) -> Result<Response, String> {
    let (host, path) = split_url(url).ok_or_else(|| format!("不支持的地址：{url}"))?;
    let agent = WideString::new(user_agent);
    let host = WideString::new(host);
    let path = WideString::new(path);
    let extra: Vec<u16> = headers.encode_utf16().collect();
    // SAFETY: 所有字符串在调用期间有效；句柄由 Handle 管理。
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
        WinHttpSetTimeouts(
            session.0,
            CONNECT_TIMEOUT_MS,
            CONNECT_TIMEOUT_MS,
            CONNECT_TIMEOUT_MS,
            RECEIVE_TIMEOUT_MS,
        )
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
        Ok(Response {
            request,
            _connection: connection,
            _session: session,
            status,
        })
    }
}

/// "https://host/path?q" → ("host", "/path?q")。
fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, "/"),
    };
    (!host.is_empty() && !host.contains([':', '@'])).then_some((host, path))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_urls_are_accepted() {
        assert_eq!(
            split_url("https://api.github.com/repos/x/y?a=1"),
            Some(("api.github.com", "/repos/x/y?a=1"))
        );
        assert_eq!(split_url("https://github.com"), Some(("github.com", "/")));
        assert_eq!(split_url("http://github.com/x"), None);
        assert_eq!(split_url("https://user@evil.com/x"), None);
        assert_eq!(split_url("https://github.com:8443/x"), None);
        assert_eq!(split_url("https:///x"), None);
    }
}
