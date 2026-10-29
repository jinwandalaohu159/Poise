//! 下载安装程序到 %LOCALAPPDATA%\Poise\updates：先写到 .part 文件，校验通过才算下载完成。
//! 同一版本已经下载过且校验通过时直接用；其他版本的安装程序随后删除。

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use poise_update::release::installer_name;
use poise_update::{Release, http};

use super::progress::{Failure, Shared, Stage};
use super::verify::{self, VerifiedFile};

pub fn fetch(release: &Release, dir: &Path, shared: &Shared) -> Result<VerifiedFile, Failure> {
    std::fs::create_dir_all(dir).map_err(|e| Failure::Network(format!("创建目录失败：{e}")))?;
    let path = dir.join(installer_name(&release.version));
    let total = release.installer_size;
    if let Some(file) = verify::open_verified(&path, total, &release.sha256) {
        log::info!("{} 已经下载过", path.display());
        shared.set_stage(Stage::Downloading { done: total, total });
        return Ok(file);
    }
    let partial = partial_path(&path);
    download(release, &partial, shared)?;
    std::fs::rename(&partial, &path).map_err(|e| Failure::Network(format!("保存失败：{e}")))?;
    let Some(file) = verify::open_verified(&path, total, &release.sha256) else {
        let _ = std::fs::remove_file(&path);
        log::warn!("下载的安装程序校验不通过，已删除");
        return Err(Failure::Verification);
    };
    delete_others(dir, &path);
    Ok(file)
}

fn download(release: &Release, partial: &Path, shared: &Shared) -> Result<(), Failure> {
    let total = release.installer_size;
    shared.set_stage(Stage::Downloading { done: 0, total });
    let mut response = http::get(&release.installer_url, &poise_update::user_agent(), "")
        .map_err(Failure::Network)?;
    if response.status() != 200 {
        return Err(Failure::Network(format!("下载返回 {}", response.status())));
    }
    let mut file = File::create(partial).map_err(|e| Failure::Network(format!("写入失败：{e}")))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        if shared.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let n = response.read(&mut buf).map_err(Failure::Network)?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > total {
            return Err(Failure::Verification);
        }
        file.write_all(&buf[..n])
            .map_err(|e| Failure::Network(format!("写入失败：{e}")))?;
        shared.set_stage(Stage::Downloading { done, total });
    }
    log::info!("下载完成：{done} 字节");
    Ok(())
}

/// "Poise-Setup-1.2.3.exe" → "Poise-Setup-1.2.3.exe.part"。
fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// 删除其他版本的安装程序和没下载完的文件。删不掉（例如正在使用）时下次再删。
fn delete_others(dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("Poise-Setup-"));
        if ours && path != keep {
            let _ = std::fs::remove_file(&path);
        }
    }
}
