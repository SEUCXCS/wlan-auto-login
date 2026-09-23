//! Remove only known application files; never recursively delete shared directories.
use anyhow::{Context, Result, ensure};
use std::{fs, io::ErrorKind, path::Path};

fn app_root(path: &Path) -> Option<&Path> {
    path.ancestors().find(|part| {
        part.file_name()
            .is_some_and(|name| name == "cxxy-wlanlogin" || name == "org.seucxcs.cxxy-wlanlogin")
    })
}

pub fn validate_directory(path: &Path) -> Result<()> {
    let root = app_root(path).unwrap_or(path);
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "拒绝通过非目录或符号链接清理应用数据：{}",
                    part.display()
                );
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        "拒绝通过重解析点清理应用数据：{}",
                        part.display()
                    );
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("无法检查目录：{}", part.display()));
            }
        }
        if part == root {
            break;
        }
    }
    Ok(())
}

pub fn remove_file(path: &Path) -> Result<()> {
    validate_directory(path.parent().context("清理路径没有父目录")?)?;
    // remove_file unlinks a leaf symlink instead of following its target.
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法删除文件：{}", path.display())),
    }
}

pub fn prune_empty_directories(path: &Path) -> Result<()> {
    validate_directory(path)?;
    let root = app_root(path).unwrap_or(path);
    for part in path.ancestors() {
        match fs::remove_dir(part) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => break,
            Err(error) => {
                return Err(error).with_context(|| format!("无法移除空目录：{}", part.display()));
            }
        }
        if part == root {
            break;
        }
    }
    Ok(())
}
