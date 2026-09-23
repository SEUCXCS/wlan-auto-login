use super::*;
use crate::wifi::WifiConnection;
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
            .ok_or_else(|| anyhow::anyhow!("unexpected Wi-Fi check"))?
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
            .ok_or_else(|| anyhow::anyhow!("credentials not saved"))?;
        assert_eq!(saved.username, "student");
        assert_eq!(saved.password, "test-secret");
    }
    assert_eq!(Preferences::load(&paths)?.line, Line::Free2);
    assert!(!crate::control::State::load(&paths)?.paused);
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
            assert_local_save(TestWifi::new([Ok(None), Ok(None)]), install, reuse_saved).await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn unavailable_wifi_identification_saves_locally_without_portal_requests() -> Result<()> {
    for install in [false, true] {
        let wifi = TestWifi::new([
            Err(anyhow::anyhow!("mock SSID permission denied")),
            Err(anyhow::anyhow!("mock SSID permission denied")),
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
        let result = run_with_store(&client, &paths, explicit_args(), install, &wifi, &store).await;
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
            anyhow::ensure!(request.len() < 16_384, "oversized mock request");
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
        Ok::<_, anyhow::Error>(())
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
            Ok::<_, anyhow::Error>(())
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
        crate::control::State {
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
        assert!(!crate::control::State::load(&paths)?.paused);
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
