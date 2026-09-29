use super::{filesystem::write_private, paths::Paths};
use anyhow::{Context, Result};
use cxxy_wlanlogin_lib::Line;
use serde::{Deserialize, Serialize};
use std::fs;

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
