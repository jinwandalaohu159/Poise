//! 记在数据目录里的散热接管：Poise 把散热模式换成别的时，用户原来的模式（机型接口的原始取值）。
//! 服务异常退出（崩溃、断电、被强行结束）来不及还原时，下次启动按它还原，而不是把 Poise 留下的模式
//! 当成用户原来的。回到原来的模式后删掉。

use std::path::Path;

use poise_platform::persist::storage::JsonFile;

const FILE: &str = "thermal.json";

pub(crate) struct Takeover {
    file: JsonFile<u8>,
    /// 文件里记着的原来的模式。
    saved: Option<u8>,
}

impl Takeover {
    /// 打开记录，连同上次没来得及还原的原来的模式。
    pub fn open(data_dir: &Path) -> (Self, Option<u8>) {
        let file = JsonFile::new(data_dir.join(FILE));
        let leftover = file.load();
        (
            Self {
                file,
                saved: leftover,
            },
            leftover,
        )
    }

    /// Poise 换掉了原来的模式（`original`）：记下，已经记着同样的就不再写。
    pub fn hold(&mut self, original: u8) {
        if self.saved == Some(original) {
            return;
        }
        match self.file.save(&original) {
            Ok(()) => self.saved = Some(original),
            Err(e) => log::warn!("记下原来的散热模式失败：{e}"),
        }
    }

    /// 回到了原来的模式：删掉记录。删不掉时留着（下次再删），不当作已经删了。
    pub fn release(&mut self) {
        if self.saved.is_none() {
            return;
        }
        match std::fs::remove_file(self.file.path()) {
            Ok(()) => self.saved = None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.saved = None,
            Err(e) => log::warn!("删掉散热接管记录失败：{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_original_survives_until_released() {
        let dir = std::env::temp_dir().join(format!("poise-takeover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut t, leftover) = Takeover::open(&dir);
        assert_eq!(leftover, None);
        t.hold(0xA0);
        // 异常退出：下次启动拿到原来的模式。
        let (mut again, leftover) = Takeover::open(&dir);
        assert_eq!(leftover, Some(0xA0));
        again.release();
        assert_eq!(Takeover::open(&dir).1, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
