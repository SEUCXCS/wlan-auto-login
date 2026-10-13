use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use crate::infra::{filesystem::write_private, paths::Paths};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub paused: bool,
    pub session: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub credential_generation: u64,
}

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("control.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context(Message::InvalidDaemonControlState),
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
    /// Caller holds the selected scope's operation lock across invalidation and mutation.
    pub fn invalidate_credentials(paths: &Paths) -> Result<()> {
        let mut state = Self::load(paths)?;
        state.credential_generation = state
            .credential_generation
            .checked_add(1)
            .ok_or_else(|| crate::infra::error::error!(Message::CredentialGenerationExhausted))?;
        state.save(paths)
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
            credential_generation: 0,
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
    #[test]
    fn credential_invalidation_is_independent_of_pause_and_scheduler_revision() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        State {
            paused: true,
            session: Some("wifi".into()),
            revision: 7,
            credential_generation: 9,
        }
        .save(&paths)?;
        State::invalidate_credentials(&paths)?;
        let mut state = State::load(&paths)?;
        assert!(state.paused);
        assert_eq!(state.session.as_deref(), Some("wifi"));
        assert_eq!(state.revision, 7);
        assert_eq!(state.credential_generation, 10);
        state.resume();
        state.save(&paths)?;
        assert_eq!(State::load(&paths)?.credential_generation, 10);
        Ok(())
    }
    #[test]
    fn absent_generation_initializes_to_zero_and_exhaustion_never_wraps() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        std::fs::write(
            paths.config.join("control.json"),
            r#"{"paused":false,"session":null,"revision":0}"#,
        )?;
        assert_eq!(State::load(&paths)?.credential_generation, 0);
        State {
            credential_generation: u64::MAX,
            ..Default::default()
        }
        .save(&paths)?;
        assert!(State::invalidate_credentials(&paths).is_err());
        assert_eq!(State::load(&paths)?.credential_generation, u64::MAX);
        Ok(())
    }
}
