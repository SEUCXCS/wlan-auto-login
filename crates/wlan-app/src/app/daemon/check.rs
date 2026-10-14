//! A single authentication check with pause/revision and network guards.
use super::policy::Policy;
use crate::infra::{
    config::Preferences,
    control,
    error::{Context, Message, Result},
    paths::Paths,
};
use crate::system::define::credentials::{BackgroundRead, CredentialPermit, Validity, WaitReason};
use crate::system::define::wifi::WifiConnection;
use std::future::Future;
use tokio::time::Instant;
use wlanlogin_lib::{AuthState, Client, LoginOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Unchanged,
    WaitingForTargetWifi,
    Paused,
    CredentialsRejected,
    CredentialsMissing,
    CredentialReadFailed(crate::infra::error::credentials::Diagnostic),
    WaitingForCredentials(WaitReason),
    WaitingForDeadline,
    Online,
    UnknownPortalState,
    WifiSessionChanged,
    Authenticated,
}

impl Outcome {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::WaitingForTargetWifi => "waiting_for_target_wifi",
            Self::Paused => "automatic_login_paused",
            Self::CredentialsRejected => "credentials_rejected",
            Self::CredentialsMissing => "credentials_missing",
            Self::CredentialReadFailed(_) => "credential_read_failed",
            Self::WaitingForCredentials(reason) => match reason {
                WaitReason::Session => "waiting_for_graphical_session",
                WaitReason::CredentialsChanged => "credentials_changed",
                WaitReason::Provider => "waiting_for_credential_provider",
                WaitReason::Locked => "waiting_for_keyring_unlock",
                WaitReason::PermissionDenied => "credential_permission_denied",
                WaitReason::NeedsInteraction => "credential_interaction_required",
            },
            Self::WaitingForDeadline => "waiting_for_deadline",
            Self::Online => "online",
            Self::UnknownPortalState => "unknown_portal_state",
            Self::WifiSessionChanged => "wifi_session_changed",
            Self::Authenticated => "authenticated",
        }
    }
    pub(super) fn diagnostic(self) -> Option<crate::infra::error::credentials::Diagnostic> {
        match self {
            Self::CredentialReadFailed(diagnostic) => Some(diagnostic),
            _ => None,
        }
    }
    pub(super) fn credentials_state(self) -> control::CredentialState {
        use control::CredentialState as S;
        match self {
            Self::CredentialsMissing => S::Missing,
            Self::CredentialReadFailed(_) => S::Failed,
            Self::WaitingForCredentials(reason) => match reason {
                WaitReason::Session => S::WaitingForSession,
                WaitReason::CredentialsChanged => S::NotChecked,
                WaitReason::Provider => S::WaitingForProvider,
                WaitReason::Locked => S::WaitingForUnlock,
                WaitReason::PermissionDenied => S::WaitingForPermission,
                WaitReason::NeedsInteraction => S::WaitingForInteraction,
            },
            Self::Authenticated | Self::CredentialsRejected => S::Available,
            _ => S::NotChecked,
        }
    }
}

pub(super) async fn tick<V>(
    client: &Client,
    paths: &Paths,
    monitor: &impl WifiConnection,
    read_credentials: impl Future<Output = Result<BackgroundRead>>,
    validate: impl FnOnce(CredentialPermit) -> V,
    policy: &mut Policy,
    scheduled: bool,
) -> Result<Outcome>
where
    V: Future<Output = Result<Validity>>,
{
    let session = monitor
        .target_session()
        .await
        .context(Message::WifiIdentificationFailed)?;
    let _operation = control::operation_lock(paths).await?;
    let mut state = control::State::load(paths)?;
    let previous_revision = policy.revision();
    let changed = policy.update(session, &mut state);
    if changed {
        state.save(paths)?;
    }
    if !scheduled && !changed && policy.revision() == previous_revision {
        return Ok(Outcome::Unchanged);
    }
    if policy.session().is_none() {
        return Ok(Outcome::WaitingForTargetWifi);
    }
    if state.paused {
        return Ok(Outcome::Paused);
    }
    if policy.credentials_rejected() {
        return Ok(Outcome::CredentialsRejected);
    }
    if !policy.should_check(&state, Instant::now()) {
        return Ok(Outcome::WaitingForDeadline);
    }
    let status = client.status().await?;
    policy.status_received();
    if status.auth == AuthState::Online {
        policy.online();
        return Ok(Outcome::Online);
    }
    if !policy.should_login(status.auth) {
        tracing::debug!("{}", Message::PortalStateUnknown);
        return Ok(Outcome::UnknownPortalState);
    }
    let saved = match read_credentials.await? {
        BackgroundRead::Available(credentials) => credentials,
        BackgroundRead::Failed(diagnostic) => {
            policy.wait_for_credentials();
            return Ok(Outcome::CredentialReadFailed(diagnostic));
        }
        BackgroundRead::Missing => {
            policy.wait_for_credentials();
            return Ok(Outcome::CredentialsMissing);
        }
        BackgroundRead::Waiting(reason) => {
            policy.wait_for_credentials();
            return Ok(Outcome::WaitingForCredentials(reason));
        }
    };
    // Recheck the Wi-Fi immediately before sending credentials after an await.
    if monitor
        .target_session()
        .await
        .context(Message::WifiIdentificationFailed)?
        .as_deref()
        != policy.session()
    {
        return Ok(Outcome::WifiSessionChanged);
    }
    // Wi-Fi inspection awaited after consumption; validate the permit once more.
    let validity = match validate(saved.permit.clone()).await {
        Ok(validity) => validity,
        Err(error) => {
            let outcome = match crate::system::flow::credentials::background::failure(&error) {
                BackgroundRead::Failed(diagnostic) => Outcome::CredentialReadFailed(diagnostic),
                BackgroundRead::Waiting(reason) => Outcome::WaitingForCredentials(reason),
                _ => unreachable!("error projection cannot contain credentials"),
            };
            policy.wait_for_credentials();
            return Ok(outcome);
        }
    };
    if validity != Validity::Valid {
        policy.wait_for_credentials();
        return Ok(Outcome::WaitingForCredentials(
            if validity == Validity::SessionExpired {
                WaitReason::Session
            } else {
                WaitReason::CredentialsChanged
            },
        ));
    }
    let line = Preferences::load(paths)?.line;
    match client.login(&saved.credentials, line).await {
        Ok(LoginOutcome::Authenticated) => {
            policy.success();
            tracing::info!(line = %line, "{}", Message::Authenticated);
            Ok(Outcome::Authenticated)
        }
        Ok(LoginOutcome::AlreadyOnline) => {
            policy.online();
            Ok(Outcome::Online)
        }
        Err(error) if error.is_credential_rejected() => {
            policy.reject_credentials();
            tracing::error!("{}", Message::CredentialsRejected);
            Ok(Outcome::CredentialsRejected)
        }
        Err(e) => Err(e.into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wlanlogin_lib::Credentials;

    struct FixedWifi;

    impl WifiConnection for FixedWifi {
        async fn target_session(&self) -> Result<Option<String>> {
            Ok(Some("student-session".into()))
        }
    }

    #[tokio::test]
    async fn daemon_handles_real_status_shape_without_login_loops() -> Result<()> {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        const ONLINE: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../wlan-api/tests/fixtures/chkstatus-online.jsonp"
        ));
        const OFFLINE: &str = r#"dr1({"result":0,"v46ip":"10.0.0.12"});"#;
        const UNKNOWN: &str = r#"dr1({"result":99,"v46ip":"10.0.0.12"});"#;
        for (responses, reads_secret, seconds, blocked, outcome) in [
            (vec![ONLINE], false, 240, false, Outcome::Online),
            (vec![UNKNOWN], false, 30, false, Outcome::UnknownPortalState),
            (vec![OFFLINE], true, 30, false, Outcome::CredentialsMissing),
            (
                vec![OFFLINE],
                true,
                30,
                false,
                Outcome::WaitingForCredentials(WaitReason::Locked),
            ),
            (
                vec![OFFLINE],
                true,
                30,
                false,
                Outcome::CredentialReadFailed(crate::infra::error::credentials::Diagnostic::new(
                    crate::infra::error::credentials::Kind::InvalidFormat,
                    crate::infra::error::credentials::Stage::Decode,
                )),
            ),
            (vec![OFFLINE, ONLINE], true, 240, false, Outcome::Online),
            (
                vec![OFFLINE, OFFLINE, r#"dr1({"result":1});"#],
                true,
                30,
                false,
                Outcome::Authenticated,
            ),
            (
                vec![OFFLINE, OFFLINE, r#"dr1({"result":0,"ret_code":1});"#],
                true,
                30,
                true,
                Outcome::CredentialsRejected,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let client = Client::new(wlanlogin_lib::PortalConfig {
                status_url: format!("{base}/status"),
                action_url: format!("{base}/action"),
                timeout: Duration::from_secs(2),
                ..Default::default()
            })?;
            let expected = responses.len();
            let server = tokio::spawn(async move {
                let mut requests = Vec::new();
                for body in responses {
                    let (mut stream, _) = listener.accept().await?;
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        request.push(stream.read_u8().await?);
                        crate::infra::error::ensure!(
                            request.len() < 16_384,
                            "oversized mock request"
                        );
                    }
                    requests.push(String::from_utf8(request)?);
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(30), listener.accept())
                        .await
                        .is_err()
                );
                Ok::<_, crate::infra::error::Error>(requests)
            });
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let mut policy = Policy::default();
            let read = std::cell::Cell::new(false);
            let result = tick(
                &client,
                &paths,
                &FixedWifi,
                async {
                    read.set(true);
                    Ok(match outcome {
                        Outcome::CredentialsMissing => BackgroundRead::Missing,
                        Outcome::WaitingForCredentials(reason) => BackgroundRead::Waiting(reason),
                        Outcome::CredentialReadFailed(diagnostic) => {
                            BackgroundRead::Failed(diagnostic)
                        }
                        _ => BackgroundRead::Available(
                            crate::system::define::credentials::BackgroundCredentials::fixture(
                                &paths,
                                Credentials {
                                    username: "student".into(),
                                    password: "test-secret".into(),
                                },
                            )?,
                        ),
                    })
                },
                |_| async { Ok(Validity::Valid) },
                &mut policy,
                true,
            )
            .await?;
            assert_eq!(result, outcome);
            assert_eq!(read.get(), reads_secret);
            let now = Instant::now();
            let wait = policy.next_deadline(now) - now;
            assert!(
                wait <= Duration::from_secs(seconds) && wait > Duration::from_secs(seconds - 1)
            );
            assert_eq!(
                policy.should_check(
                    &control::State::load(&paths)?,
                    now + Duration::from_secs(300)
                ),
                !blocked
            );
            // A wake on the same connection must retain the chosen poll deadline.
            tick(
                &client,
                &paths,
                &FixedWifi,
                async { panic!("premature credential read") },
                |_| async { Ok(Validity::Valid) },
                &mut policy,
                true,
            )
            .await?;
            let requests = tokio::time::timeout(Duration::from_secs(3), server).await??;
            assert_eq!(requests?.len(), expected);
        }
        Ok(())
    }

    struct UnavailableWifi;
    impl WifiConnection for UnavailableWifi {
        async fn target_session(&self) -> Result<Option<String>> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
        }
    }

    struct DisconnectedWifi;
    impl WifiConnection for DisconnectedWifi {
        async fn target_session(&self) -> Result<Option<String>> {
            Ok(None)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wifi_failure_retries_and_reports_pause_or_disconnect_without_portal_access()
    -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let client = Client::new(wlanlogin_lib::PortalConfig {
            status_url: format!("{base}/status"),
            action_url: format!("{base}/action"),
            ..Default::default()
        })?;
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let mut policy = Policy::default();
        let mut state = control::State {
            paused: true,
            session: Some("student-session".into()),
            revision: 0,
            credential_generation: 0,
        };
        policy.update(state.session.clone(), &mut state);
        state.save(&paths)?;
        let error = tick(
            &client,
            &paths,
            &UnavailableWifi,
            async { panic!("Wi-Fi failure must not read credentials") },
            |_| async { Ok(Validity::Valid) },
            &mut policy,
            true,
        )
        .await
        .err()
        .context("expected Wi-Fi failure")?;
        assert!(error.to_string().contains("identify the current Wi-Fi"));
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(|error| error.kind()),
            Some(std::io::ErrorKind::BrokenPipe)
        );
        let start = Instant::now();
        assert_eq!(policy.failure(start, false), Duration::from_secs(30));
        super::super::runtime::next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(30));
        assert_eq!(
            tick(
                &client,
                &paths,
                &FixedWifi,
                async { panic!("paused retry must not read credentials") },
                |_| async { Ok(Validity::Valid) },
                &mut policy,
                true,
            )
            .await?,
            Outcome::Paused
        );
        assert_eq!(
            tick(
                &client,
                &paths,
                &DisconnectedWifi,
                async { panic!("disconnected check must not read credentials") },
                |_| async { Ok(Validity::Valid) },
                &mut policy,
                true,
            )
            .await?,
            Outcome::WaitingForTargetWifi
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
        Ok(())
    }

    struct ExpiringWifi {
        calls: std::cell::Cell<u32>,
        state: tokio::sync::watch::Sender<crate::system::define::credentials::SessionState>,
        changed_wifi: bool,
    }
    impl WifiConnection for ExpiringWifi {
        async fn target_session(&self) -> Result<Option<String>> {
            let calls = self.calls.get() + 1;
            self.calls.set(calls);
            if calls == 2 {
                // Expiry happens inside the last Wi-Fi await, after credentials were consumed.
                tokio::task::yield_now().await;
                let mut next = self.state.borrow().clone();
                next.update(Some("!old".into()))?;
                self.state.send_replace(next);
                if self.changed_wifi {
                    return Ok(Some("another-wifi".into()));
                }
            }
            Ok(Some("student-session".into()))
        }
    }
    #[tokio::test]
    async fn final_checkpoint_after_wifi_await_blocks_expired_changed_and_failed_permits()
    -> Result<()> {
        use crate::system::define::credentials::{
            BackgroundCredentials, SessionIdentity, SessionPermit, SessionState,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for case in 0..4 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let client = Client::new(wlanlogin_lib::PortalConfig {
                status_url: format!("{base}/status"),
                action_url: format!("{base}/action"),
                timeout: Duration::from_secs(2),
                ..Default::default()
            })?;
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await?);
                }
                assert!(String::from_utf8(request)?.starts_with("GET /status"));
                let body = r#"dr1({"result":0,"v46ip":"10.0.0.12"});"#;
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
                Ok::<_, crate::infra::error::Error>(listener)
            });
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let initial = SessionState {
                hint: Some("old".into()),
                generation: 0,
            };
            let (sender, receiver) = tokio::sync::watch::channel(initial.clone());
            let wifi = ExpiringWifi {
                calls: std::cell::Cell::new(0),
                state: sender,
                changed_wifi: case == 3,
            };
            let mut credentials = BackgroundCredentials::fixture(
                &paths,
                Credentials {
                    username: "mock".into(),
                    password: "mock-only".into(),
                },
            )?;
            credentials.permit.session = Some(SessionPermit {
                identity: SessionIdentity {
                    id: "old".into(),
                    started_at: 1,
                },
                state: initial,
            });
            let mut policy = Policy::default();
            let outcome = tick(
                &client,
                &paths,
                &wifi,
                async { Ok(BackgroundRead::Available(credentials)) },
                |permit| async move {
                    assert!(
                        permit
                            .session
                            .is_some_and(|session| session.state != *receiver.borrow())
                    );
                    match case {
                        0 => Ok(Validity::SessionExpired),
                        1 => Ok(Validity::CredentialsChanged),
                        2 => Err(crate::infra::error::credentials::CredentialError::new(
                            crate::infra::error::credentials::Kind::Timeout,
                            crate::infra::error::credentials::Stage::Session,
                        )
                        .into()),
                        _ => panic!("changed Wi-Fi must prevent further credential validation"),
                    }
                },
                &mut policy,
                true,
            )
            .await?;
            let expected = match case {
                0 => Outcome::WaitingForCredentials(WaitReason::Session),
                1 => Outcome::WaitingForCredentials(WaitReason::CredentialsChanged),
                2 => Outcome::CredentialReadFailed(
                    crate::infra::error::credentials::Diagnostic::new(
                        crate::infra::error::credentials::Kind::Timeout,
                        crate::infra::error::credentials::Stage::Session,
                    ),
                ),
                _ => Outcome::WifiSessionChanged,
            };
            assert_eq!(outcome, expected);
            assert!(!policy.credentials_rejected());
            let listener = tokio::time::timeout(Duration::from_secs(2), server).await???;
            // Only the initial status query is allowed; even login's status query is forbidden.
            assert!(
                tokio::time::timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        }
        Ok(())
    }
}
