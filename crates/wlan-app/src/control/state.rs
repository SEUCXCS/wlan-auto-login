use crate::config::{Paths, write_private};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub paused: bool,
    pub session: Option<String>,
    pub revision: u64,
}

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("control.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("后台控制状态损坏"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("control.json"),
            &serde_json::to_vec(self)?,
        )
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pause_survives_restart_and_explicit_login_resumes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        State {
            paused: true,
            session: Some("connection-1".into()),
            revision: 0,
        }
        .save(&paths)?;
        let mut state = State::load(&paths)?;
        assert!(state.paused);
        state.resume();
        state.save(&paths)?;
        assert!(!State::load(&paths)?.paused);
        assert_eq!(State::load(&paths)?.revision, 1);
        Ok(())
    }
}
