use super::cli::LoginArgs;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, bail};
use crate::{
    infra::{config::Preferences, paths::Paths},
    system::credentials,
    system::wifi,
    tui,
};
use cxxy_wlanlogin_lib::{Client, Credentials, Line, LoginOutcome};
use std::io::{self, IsTerminal};

pub struct Form {
    pub credentials: Credentials,
    pub line: Line,
    pub remember: bool,
}

pub fn resolve(
    args: &mut LoginArgs,
    saved: Option<&Credentials>,
    default_line: Line,
    force_remember: bool,
) -> Form {
    let username = args
        .username
        .take()
        .or_else(|| saved.map(|c| c.username.clone()))
        .unwrap_or_default();
    let password = args
        .password
        .take()
        .or_else(|| {
            saved
                .filter(|c| c.username == username)
                .map(|c| c.password.clone())
        })
        .unwrap_or_default();
    Form {
        credentials: Credentials { username, password },
        line: args.line.unwrap_or(default_line),
        remember: force_remember || args.remember_me,
    }
}

pub async fn run(
    client: &Client,
    paths: &Paths,
    args: LoginArgs,
    force_remember: bool,
) -> Result<()> {
    let wifi = wifi::WifiProbe::new().await;
    run_with_wifi(client, paths, args, force_remember, &wifi).await
}

pub async fn run_with_wifi(
    client: &Client,
    paths: &Paths,
    args: LoginArgs,
    force_remember: bool,
    wifi: &impl wifi::WifiConnection,
) -> Result<()> {
    let store = credentials::ScopedStore::new(paths, args.credential_store)?;
    run_with_store(client, paths, args, force_remember, wifi, &store).await
}

async fn run_with_store(
    client: &Client,
    paths: &Paths,
    mut args: LoginArgs,
    force_remember: bool,
    wifi: &impl wifi::WifiConnection,
    store: &impl credentials::Store,
) -> Result<()> {
    let initial_wifi = wifi::wifi_state(wifi).await;
    let save_only = !initial_wifi.is_target();
    if args.password_stdin {
        let mut password = String::new();
        io::stdin()
            .read_line(&mut password)
            .context(Message::FailedToReadPasswordFromStdin)?;
        if password.ends_with('\n') {
            password.pop();
            if password.ends_with('\r') {
                password.pop();
            }
        }
        args.password = Some(password);
    }
    let mut warning = initial_wifi.notice();
    let saved = if args.username.is_none() || args.password.is_none() {
        match store.load().await {
            Ok(value) => value,
            Err(e) => {
                warning = Some(e.to_string());
                None
            }
        }
    } else {
        None
    };
    let prefs = Preferences::load(paths)?;
    let mut form = resolve(
        &mut args,
        saved.as_ref(),
        prefs.line,
        force_remember || save_only,
    );
    let interactive = wants_tui(&args, &form);
    if interactive
        && (args.non_interactive || !io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        bail!(Message::CredentialsAreMissingProvideArgumentsCXXYWLAN {
            value0: (warning.map(|s| format!("；{s}")).unwrap_or_default()).to_string()
        });
    }
    loop {
        if interactive {
            let remember_reason =
                if force_remember && paths.scope == crate::infra::scope::Scope::System {
                    Some("安装系统服务必须保存凭据，供本机开机自动登录使用。")
                } else if force_remember {
                    Some("安装服务必须保存账号和密码。")
                } else if save_only {
                    Some("离线配置仅保存账号和密码。")
                } else {
                    None
                };
            form = tui::prompt(form, remember_reason, warning.take())?;
        }
        let _operation = crate::infra::control::operation_lock(paths).await?;
        match submit(client, &form, wifi).await {
            Ok(outcome) => {
                let authenticated = outcome.is_some();
                let remember = form.remember || !authenticated;
                if remember {
                    store
                        .save(form.credentials.clone())
                        .await
                        .context(if authenticated {
                            "Authenticated but failed to save credentials; service installation cannot continue"
                        } else {
                            "Failed to save offline credentials; service installation cannot continue"
                        })?;
                }
                Preferences { line: form.line }.save(paths)?;
                let mut state = crate::infra::control::State::load(paths)?;
                state.resume();
                state.save(paths)?;
                crate::infra::control::wake(paths).await;
                if outcome == Some(LoginOutcome::AlreadyOnline) {
                    tracing::info!(credentials_saved = remember, "{}", Message::AlreadyOnline);
                } else if authenticated {
                    tracing::info!(line = %form.line, credentials_saved = remember, "{}", Message::Authenticated);
                } else {
                    tracing::info!("{}", Message::CredentialsSavedOffline);
                }
                return Ok(());
            }
            Err(e) if interactive => {
                warning = Some(format!("{e:#}"));
            }
            Err(e) => return Err(e),
        }
    }
}

async fn submit(
    client: &Client,
    form: &Form,
    wifi: &impl wifi::WifiConnection,
) -> Result<Option<LoginOutcome>> {
    // Credential entry, keychain access and the operation lock may all take time.
    // Re-read the SSID before Client::login sends even its initial status request.
    let state = wifi::wifi_state(wifi).await;
    if !state.is_target() {
        if let Some(notice) = state.notice() {
            tracing::warn!("{notice}");
        }
        return Ok(None);
    }
    Ok(Some(client.login(&form.credentials, form.line).await?))
}

fn wants_tui(args: &LoginArgs, form: &Form) -> bool {
    args.interactive
        || form.credentials.username.trim().is_empty()
        || form.credentials.password.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::wifi::WifiConnection;
    use cxxy_wlanlogin_lib::PortalConfig;
    use std::{cell::RefCell, collections::VecDeque, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::timeout,
    };

    struct TestWifi(RefCell<VecDeque<Result<Option<String>>>>);

    impl TestWifi {
        fn new(states: impl IntoIterator<Item = Result<Option<String>>>) -> Self {
            Self(RefCell::new(states.into_iter().collect()))
        }
    }

    impl WifiConnection for TestWifi {
        async fn target_session(&self) -> Result<Option<String>> {
            self.0
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| crate::infra::error::error!("unexpected Wi-Fi check"))?
        }
    }

    #[derive(Default)]
    struct TestStore {
        saved: RefCell<Option<Credentials>>,
        fail_save: bool,
    }

    impl credentials::Store for TestStore {
        async fn load(&self) -> Result<Option<Credentials>> {
            Ok(self.saved.borrow().clone())
        }

        async fn save(&self, value: Credentials) -> Result<()> {
            if self.fail_save {
                bail!("mock keyring is locked");
            }
            *self.saved.borrow_mut() = Some(value);
            Ok(())
        }
    }

    fn explicit_args() -> LoginArgs {
        LoginArgs {
            username: Some("student".into()),
            password: Some("test-secret".into()),
            line: Some(Line::Free2),
            non_interactive: true,
            ..Default::default()
        }
    }

    async fn mock_portal() -> Result<(Client, TcpListener)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let client = Client::new(PortalConfig {
            status_url: format!("{base}/status"),
            action_url: format!("{base}/action"),
            timeout: Duration::from_secs(1),
            ..PortalConfig::default()
        })?;
        Ok((client, listener))
    }

    async fn assert_local_save(wifi: TestWifi, install: bool, reuse_saved: bool) -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        let (client, listener) = mock_portal().await?;
        let store = TestStore::default();
        let args = if reuse_saved {
            *store.saved.borrow_mut() = Some(Credentials {
                username: "student".into(),
                password: "test-secret".into(),
            });
            LoginArgs {
                line: Some(Line::Free2),
                non_interactive: true,
                ..Default::default()
            }
        } else {
            explicit_args()
        };
        run_with_store(&client, &paths, args, install, &wifi, &store).await?;
        {
            let saved = store.saved.borrow();
            let saved = saved
                .as_ref()
                .ok_or_else(|| crate::infra::error::error!("credentials not saved"))?;
            assert_eq!(saved.username, "student");
            assert_eq!(saved.password, "test-secret");
        }
        assert_eq!(Preferences::load(&paths)?.line, Line::Free2);
        assert!(!crate::infra::control::State::load(&paths)?.paused);
        assert!(
            wifi.0.borrow().is_empty(),
            "must recheck after resolving credentials"
        );
        assert!(
            timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err(),
            "offline login must not send even a status request"
        );
        Ok(())
    }

    #[tokio::test]
    async fn off_target_login_and_install_save_without_any_portal_request() -> Result<()> {
        for install in [false, true] {
            for reuse_saved in [false, true] {
                assert_local_save(TestWifi::new([Ok(None), Ok(None)]), install, reuse_saved)
                    .await?;
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn unavailable_wifi_identification_saves_locally_without_portal_requests() -> Result<()> {
        for install in [false, true] {
            let wifi = TestWifi::new([
                Err(crate::infra::error::error!("mock SSID permission denied")),
                Err(crate::infra::error::error!("mock SSID permission denied")),
            ]);
            assert_local_save(wifi, install, false).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn leaving_target_wifi_before_submission_switches_to_local_save() -> Result<()> {
        for install in [false, true] {
            let wifi = TestWifi::new([Ok(Some("student-session".into())), Ok(None)]);
            assert_local_save(wifi, install, false).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn offline_save_failure_aborts_login_and_install_preparation() -> Result<()> {
        for install in [false, true] {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path().join("config"));
            let (client, listener) = mock_portal().await?;
            let store = TestStore {
                fail_save: true,
                ..Default::default()
            };
            let wifi = TestWifi::new([Ok(None), Ok(None)]);
            let result =
                run_with_store(&client, &paths, explicit_args(), install, &wifi, &store).await;
            assert!(result.is_err());
            assert!(store.saved.borrow().is_none());
            assert!(!paths.config.join("preferences.json").exists());
            assert!(
                timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        }
        Ok(())
    }

    async fn reply(listener: &TcpListener, body: &str) -> Result<String> {
        let (mut stream, _) = timeout(Duration::from_secs(2), listener.accept()).await??;
        let mut request = Vec::new();
        timeout(Duration::from_secs(2), async {
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await?);
                crate::infra::error::ensure!(request.len() < 16_384, "oversized mock request");
            }
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await?;
            Ok::<_, crate::infra::error::Error>(())
        })
        .await??;
        Ok(String::from_utf8(request)?)
    }

    #[tokio::test]
    async fn target_wifi_authenticates_and_saves_only_when_requested_or_installing() -> Result<()> {
        for (remember, install) in [(false, false), (true, false), (false, true)] {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path().join("config"));
            let (client, listener) = mock_portal().await?;
            let store = TestStore::default();
            let wifi = TestWifi::new([
                Ok(Some("student-session".into())),
                Ok(Some("student-session".into())),
            ]);
            let args = LoginArgs {
                remember_me: remember,
                ..explicit_args()
            };
            let serve = async {
                let status = reply(&listener, r#"dr1({"v46ip":"10.0.0.12","online":0});"#).await?;
                let login = reply(&listener, r#"dr1({"result":1});"#).await?;
                assert!(status.starts_with("GET /status?"));
                assert!(login.starts_with("GET /action?"));
                assert!(login.contains("&a=login&"));
                Ok::<_, crate::infra::error::Error>(())
            };
            let (result, served) = tokio::join!(
                run_with_store(&client, &paths, args, install, &wifi, &store),
                serve
            );
            served?;
            result?;
            assert_eq!(store.saved.borrow().is_some(), remember || install);
            assert!(wifi.0.borrow().is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn already_online_login_resumes_daemon_without_resubmitting_credentials() -> Result<()> {
        for remember in [false, true] {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let (client, listener) = mock_portal().await?;
            let store = TestStore::default();
            let wifi = TestWifi::new([
                Ok(Some("student-session".into())),
                Ok(Some("student-session".into())),
            ]);
            crate::infra::control::State {
                paused: true,
                ..Default::default()
            }
            .save(&paths)?;
            let args = LoginArgs {
                remember_me: remember,
                ..explicit_args()
            };
            let (result, served) = tokio::join!(
                run_with_store(&client, &paths, args, false, &wifi, &store),
                reply(
                    &listener,
                    r#"dr1({"result":1,"v46ip":"10.0.0.12","uid":"student"});"#
                ),
            );
            assert!(served?.starts_with("GET /status?"));
            result?;
            assert_eq!(store.saved.borrow().is_some(), remember);
            assert!(!crate::infra::control::State::load(&paths)?.paused);
            assert!(
                timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn saved_credentials_skip_tui_unless_requested() {
        let saved = Credentials {
            username: "student".into(),
            password: "secret".into(),
        };
        let mut args = LoginArgs::default();
        let form = resolve(&mut args, Some(&saved), Line::Free1, false);
        assert!(!wants_tui(&args, &form));
        args.interactive = true;
        assert!(wants_tui(&args, &form));
    }
    #[test]
    fn changing_account_does_not_reuse_another_accounts_password() {
        let saved = Credentials {
            username: "old".into(),
            password: "old-secret".into(),
        };
        let mut args = LoginArgs {
            username: Some("new".into()),
            ..Default::default()
        };
        let form = resolve(&mut args, Some(&saved), Line::Free2, false);
        assert!(form.credentials.password.is_empty());
        assert_eq!(form.line, Line::Free2);
        assert!(!form.remember);
    }
    #[test]
    fn install_forces_remember_even_without_the_flag() {
        let form = resolve(&mut LoginArgs::default(), None, Line::Free1, true);
        assert!(form.remember);
    }
    #[test]
    fn explicit_credentials_override_saved_values() {
        let saved = Credentials {
            username: "old".into(),
            password: "old-secret".into(),
        };
        let mut args = LoginArgs {
            username: Some("new".into()),
            password: Some("new-secret".into()),
            line: Some(Line::Campus),
            ..Default::default()
        };
        let form = resolve(&mut args, Some(&saved), Line::Free1, false);
        assert_eq!(form.credentials.password, "new-secret");
        assert_eq!(form.line, Line::Campus);
    }
}
