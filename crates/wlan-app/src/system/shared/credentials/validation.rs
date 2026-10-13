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
    #[cfg(target_os = "linux")]
    if paths.scope == crate::infra::scope::Scope::User {
        let Some(session_permit) = &permit.session else {
            return Ok(Validity::SessionExpired);
        };
        if !crate::system::linux::runtime::session::valid(session_permit, session).await? {
            return Ok(Validity::SessionExpired);
        }
    }
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
    use crate::infra::scope::Scope;
    use wlanlogin_lib::Credentials;

    impl crate::system::define::credentials::BackgroundCredentials {
        pub(crate) fn fixture(
            paths: &crate::infra::paths::Paths,
            credentials: Credentials,
        ) -> Result<Self> {
            Ok(Self {
                credentials,
                permit: snapshot(paths)?,
            })
        }
    }

    #[tokio::test]
    async fn generations_are_scope_local_and_system_reads_ignore_desktop_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let user = Paths::test(dir.path().join("user"));
        let mut system = Paths::test(dir.path().join("system"));
        system.scope = Scope::System;
        let permit = snapshot(&system)?;
        let (sender, receiver) = tokio::sync::watch::channel(SessionState::default());
        State::invalidate_credentials(&user)?;
        let mut state = receiver.borrow().clone();
        state.update(Some("!old-desktop".into()))?;
        sender.send_replace(state);
        assert_eq!(
            validate(&system, &permit, &receiver).await?,
            Validity::Valid
        );
        State::invalidate_credentials(&system)?;
        assert_eq!(
            validate(&system, &permit, &receiver).await?,
            Validity::CredentialsChanged
        );
        assert_eq!(State::load(&user)?.credential_generation, 1);
        Ok(())
    }
    #[tokio::test]
    async fn permits_are_bound_to_scope_and_config_root() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path().join("one"));
        paths.scope = Scope::System;
        let permit = snapshot(&paths)?;
        let receiver = tokio::sync::watch::channel(SessionState::default()).1;
        let mut wrong = paths.clone();
        wrong.scope = Scope::User;
        assert!(validate(&wrong, &permit, &receiver).await.is_err());
        wrong = Paths::test(dir.path().join("two"));
        wrong.scope = Scope::System;
        assert!(validate(&wrong, &permit, &receiver).await.is_err());
        Ok(())
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn linux_user_permit_without_session_is_rejected_without_bus_access() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let permit = snapshot(&paths)?;
        let receiver = tokio::sync::watch::channel(SessionState::default()).1;
        assert_eq!(
            validate(&paths, &permit, &receiver).await?,
            Validity::SessionExpired
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
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
