use anyhow::{Context, Result};
use cxxy_wlanlogin_lib::Line;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Paths {
    pub config: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
            .context("无法确定用户配置目录")?;
        Ok(Self {
            config: dirs.config_dir().into(),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Preferences {
    pub line: Line,
}

impl Preferences {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("preferences.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("线路配置损坏"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("无法读取线路配置"),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("文件没有父目录")?;
    private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
