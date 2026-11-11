//! Validate result freshness independently of credential reads or provider activation.
use crate::infra::{
    control::State,
    error::{Message, Result, ensure},
    paths::Paths,
};
use crate::system::define::credentials::{CredentialPermit, SessionState, Validity};

pub(crate) fn snapshot(paths: &Paths) -> Result<CredentialPermit> {
    let generation = State::load(paths)?.credential_generation;
    Ok(CredentialPermit {
        scope: paths.scope,
        config: paths.config.clone(),
        generation,
        session: None,
    })
}

pub(crate) async fn validate(
    paths: &Paths,
    permit: &CredentialPermit,
    session: &tokio::sync::watch::Receiver<SessionState>,
) -> Result<Validity> {
    // Scope binding prevents a user permit from authorizing system storage or another root.
    ensure!(
        permit.scope == paths.scope && permit.config == paths.config,
        Message::CredentialPermitScopeMismatch
    );
    if State::load(paths)?.credential_generation != permit.generation {
        return Ok(Validity::CredentialsChanged);
    }

    // Only Linux user storage imposes the graphical-session prerequisite.

    let _ = session;

    // A mutation while native session queries awaited also invalidates the result.
    if State::load(paths)?.credential_generation != permit.generation {
        return Ok(Validity::CredentialsChanged);
    }
    Ok(Validity::Valid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::Result;

    #[tokio::test]
    async fn native_user_permits_need_no_linux_graphical_session() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let permit = snapshot(&paths)?;
        let receiver = tokio::sync::watch::channel(SessionState::default()).1;
        assert_eq!(validate(&paths, &permit, &receiver).await?, Validity::Valid);
        State::invalidate_credentials(&paths)?;
        assert_eq!(
            validate(&paths, &permit, &receiver).await?,
            Validity::CredentialsChanged
        );
        Ok(())
    }
}
