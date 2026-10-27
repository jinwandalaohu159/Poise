//! 命名管道客户端：一问一答，每条消息一行 JSON。

use std::fs::{File, OpenOptions};
use std::io::BufReader;

use poise_protocol::{
    PIPE_NAME, PROTOCOL_VERSION, Request, RequestEnvelope, ResponseEnvelope, read_message,
    write_message,
};
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub enum CallError {
    /// 连接断开或服务没在运行。
    Disconnected(String),
    /// 服务拒绝了请求。
    Rejected(String),
}

impl CallError {
    pub fn message(&self) -> &str {
        match self {
            Self::Disconnected(m) | Self::Rejected(m) => m,
        }
    }
}

pub struct PipeClient {
    writer: File,
    reader: BufReader<File>,
    next_id: u64,
}

impl PipeClient {
    pub fn connect() -> Result<Self, CallError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(PIPE_NAME)
            .map_err(|e| CallError::Disconnected(e.to_string()))?;
        let reader = BufReader::new(
            file.try_clone()
                .map_err(|e| CallError::Disconnected(e.to_string()))?,
        );
        let mut client = Self {
            writer: file,
            reader,
            next_id: 1,
        };
        client.call::<serde_json::Value>(Request::Hello {
            protocol: PROTOCOL_VERSION,
        })?;
        Ok(client)
    }

    pub fn call<T: DeserializeOwned>(&mut self, request: Request) -> Result<T, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let disconnected = |e: poise_protocol::FramingError| CallError::Disconnected(e.to_string());
        write_message(&mut self.writer, &RequestEnvelope { id, request }).map_err(disconnected)?;
        let response: ResponseEnvelope = read_message(&mut self.reader)
            .map_err(disconnected)?
            .ok_or_else(|| CallError::Disconnected("服务关闭了连接".into()))?;
        if !response.ok {
            return Err(CallError::Rejected(response.error.unwrap_or_default()));
        }
        serde_json::from_value(response.result.unwrap_or(serde_json::Value::Null))
            .map_err(|e| CallError::Rejected(e.to_string()))
    }
}
