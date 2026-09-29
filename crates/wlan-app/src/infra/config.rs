use super::{filesystem::write_private, paths::Paths};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
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
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context(Message::InvalidConnectionPreferences)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context(Message::FailedToReadConnectionPreferences),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}
