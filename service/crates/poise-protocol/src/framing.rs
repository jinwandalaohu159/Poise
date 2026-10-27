//! 换行分隔的 JSON 读写，带长度上限，防止恶意客户端用超大消息耗尽内存。

use std::io::{self, BufRead, Write};

use serde::{Serialize, de::DeserializeOwned};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FramingError {
    Io(io::Error),
    TooLarge,
    Json(serde_json::Error),
}

impl std::fmt::Display for FramingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::TooLarge => write!(f, "消息超过 {MAX_MESSAGE_BYTES} 字节"),
            Self::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for FramingError {}

/// 读取一条消息；连接正常关闭时返回 `Ok(None)`。
pub fn read_message<T: DeserializeOwned>(r: &mut impl BufRead) -> Result<Option<T>, FramingError> {
    let mut buf = Vec::new();
    loop {
        let chunk = r.fill_buf().map_err(FramingError::Io)?;
        if chunk.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(FramingError::Io(io::ErrorKind::UnexpectedEof.into()))
            };
        }
        match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => {
                buf.extend_from_slice(&chunk[..i]);
                r.consume(i + 1);
                break;
            }
            None => {
                let n = chunk.len();
                buf.extend_from_slice(chunk);
                r.consume(n);
            }
        }
        if buf.len() > MAX_MESSAGE_BYTES {
            return Err(FramingError::TooLarge);
        }
    }
    if buf.len() > MAX_MESSAGE_BYTES {
        return Err(FramingError::TooLarge);
    }
    serde_json::from_slice(&buf)
        .map(Some)
        .map_err(FramingError::Json)
}

pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), FramingError> {
    let mut bytes = serde_json::to_vec(msg).map_err(FramingError::Json)?;
    bytes.push(b'\n');
    w.write_all(&bytes).map_err(FramingError::Io)?;
    w.flush().map_err(FramingError::Io)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn reads_consecutive_messages() {
        let mut out = Vec::new();
        write_message(&mut out, &1u32).unwrap();
        write_message(&mut out, &"two").unwrap();
        let mut r = Cursor::new(out);
        assert_eq!(read_message::<u32>(&mut r).unwrap(), Some(1));
        assert_eq!(read_message::<String>(&mut r).unwrap(), Some("two".into()));
        assert!(read_message::<u32>(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_messages() {
        let big = vec![b'a'; MAX_MESSAGE_BYTES + 10];
        let mut r = Cursor::new(big);
        assert!(matches!(
            read_message::<String>(&mut r),
            Err(FramingError::TooLarge)
        ));
    }
}
