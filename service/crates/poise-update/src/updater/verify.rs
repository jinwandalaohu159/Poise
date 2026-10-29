//! 校验下载的安装程序：大小和 SHA-256 都与发布信息一致。
//!
//! 校验通过的文件保持打开（只允许别人读）：从校验到安装程序启动之间，文件不会被改写或替换。

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use windows::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHashData,
};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

/// 校验通过、保持打开的安装程序。
pub struct VerifiedFile {
    _file: File,
    pub path: PathBuf,
}

/// 打开并校验；不存在、大小或 SHA-256（小写十六进制）不一致时返回 None。
pub fn open_verified(path: &Path, size: u64, sha256: &str) -> Option<VerifiedFile> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)
        .ok()?;
    if file.metadata().ok()?.len() != size || sha256_of(&file)? != sha256 {
        return None;
    }
    Some(VerifiedFile {
        _file: file,
        path: path.to_path_buf(),
    })
}

/// 文件内容的 SHA-256（小写十六进制），用系统自带的 CNG 计算。
fn sha256_of(mut file: &File) -> Option<String> {
    let mut hash = BCRYPT_HASH_HANDLE::default();
    // SAFETY: 使用系统预置的 SHA-256 算法句柄，哈希对象由系统分配。
    unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut hash, None, None, 0) }
        .ok()
        .ok()?;
    let digest = (|| {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = file.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            // SAFETY: 哈希句柄有效，输入是刚读到的数据。
            unsafe { BCryptHashData(hash, &buf[..n], 0) }.ok().ok()?;
        }
        let mut out = [0u8; 32];
        // SAFETY: SHA-256 的结果是 32 字节。
        unsafe { BCryptFinishHash(hash, &mut out, 0) }.ok().ok()?;
        Some(out.iter().map(|b| format!("{b:02x}")).collect())
    })();
    // SAFETY: 句柄由本函数创建，只释放一次。
    let _ = unsafe { BCryptDestroyHash(hash) };
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "abc" 的 SHA-256（FIPS 180-2 的标准测试向量）。
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn checks_size_and_digest_and_locks_the_file() {
        let dir = std::env::temp_dir().join(format!("poise-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("setup.exe");
        std::fs::write(&path, b"abc").unwrap();

        let verified = open_verified(&path, 3, ABC).expect("应当通过");
        // 校验通过后文件被占着，不能改写。
        assert!(std::fs::write(&path, b"xyz").is_err());
        drop(verified);

        assert!(open_verified(&path, 4, ABC).is_none());
        assert!(open_verified(&path, 3, &"0".repeat(64)).is_none());
        assert!(open_verified(&dir.join("missing.exe"), 3, ABC).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
