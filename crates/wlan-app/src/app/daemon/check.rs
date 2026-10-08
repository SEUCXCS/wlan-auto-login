//! A single authentication check with pause/revision and network guards.
use super::policy::Policy;
use crate::infra::{
    config::Preferences,
    control,
    error::{Context, Message, Result},
    paths::Paths,
};
use crate::system::define::wifi::WifiConnection;
use std::future::Future;
use tokio::time::Instant;
use wlanlogin_lib::{AuthState, Client, Credentials, LoginOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Unchanged,
    WaitingForTargetWifi,
    Paused,
    CredentialsRejected,
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
            Self::WaitingForDeadline => "waiting_for_deadline",
            Self::Online => "online",
            Self::UnknownPortalState => "unknown_portal_state",
            Self::WifiSessionChanged => "wifi_session_changed",
            Self::Authenticated => "authenticated",
        }
    }
}

pub(super) async fn tick(
    client: &Client,
    paths: &Paths,
    monitor: &impl WifiConnection,
    read_credentials: impl Future<Output = Result<Option<Credentials>>>,
    policy: &mut Policy,
    scheduled: bool,
) -> Result<Outcome> {
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
    let saved = read_credentials
        .await?
        .ok_or_else(|| crate::infra::error::error!(Message::SavedCredentialsMissing))?;
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
    let line = Preferences::load(paths)?.line;
    match client.login(&saved, line).await {
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
                    Ok(Some(Credentials {
                        username: "student".into(),
                        password: "test-secret".into(),
                    }))
                },
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
        };
        policy.update(state.session.clone(), &mut state);
        state.save(&paths)?;
        let error = tick(
            &client,
            &paths,
            &UnavailableWifi,
            async { panic!("Wi-Fi failure must not read credentials") },
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
}
