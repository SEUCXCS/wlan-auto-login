use super::*;
use std::{cell::Cell, fs};

#[tokio::test]
async fn forget_removes_local_credentials_even_when_remote_logout_and_stop_fail() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths = Paths::test(dir.path());
    let secret = dir.path().join("test-secret");
    fs::write(&secret, "mock secret")?;
    let result = run_with(
        &paths,
        true,
        async { anyhow::bail!("portal unavailable") },
        async {
            fs::remove_file(&secret)?;
            Ok(())
        },
        async { anyhow::bail!("stop failed") },
    )
    .await;
    let message = format!("{:#}", result.err().context("expected partial failure")?);
    assert!(message.contains("portal unavailable") && message.contains("stop failed"));
    assert!(!secret.exists());
    assert!(control::State::load(&paths)?.paused);
    Ok(())
}

#[tokio::test]
async fn forgetting_failure_keeps_automatic_login_paused_and_reports_failure() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths = Paths::test(dir.path());
    let stopped = Cell::new(false);
    let result = run_with(
        &paths,
        true,
        async { Ok(()) },
        async { anyhow::bail!("keyring locked") },
        async {
            stopped.set(true);
            Ok(())
        },
    )
    .await;
    assert!(
        format!("{:#}", result.err().context("expected delete failure")?)
            .contains("keyring locked")
    );
    assert!(stopped.get());
    assert!(control::State::load(&paths)?.paused);
    Ok(())
}

#[tokio::test]
async fn ordinary_logout_preserves_credentials_and_restores_state_on_remote_failure() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let paths = Paths::test(dir.path());
    let result = run_with(
        &paths,
        false,
        async { anyhow::bail!("portal unavailable") },
        async { panic!("ordinary logout must not delete credentials") },
        async { panic!("ordinary logout must not stop the daemon") },
    )
    .await;
    assert!(result.is_err());
    assert!(!control::State::load(&paths)?.paused);
    run_with(
        &paths,
        false,
        async { Ok(()) },
        async { panic!("ordinary logout must not delete credentials") },
        async { panic!("ordinary logout must not stop the daemon") },
    )
    .await?;
    assert!(control::State::load(&paths)?.paused);
    Ok(())
}

struct OfflineWifi;
impl WifiConnection for OfflineWifi {
    async fn target_session(&self) -> Result<Option<String>> {
        Ok(None)
    }
}

#[tokio::test]
async fn offline_forget_never_contacts_the_portal() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let client = Client::new(cxxy_wlanlogin_lib::PortalConfig {
        status_url: format!("{base}/status"),
        action_url: format!("{base}/action"),
        ..Default::default()
    })?;
    let dir = tempfile::tempdir()?;
    let paths = Paths::test(dir.path());
    let deleted = Cell::new(false);
    run_with(
        &paths,
        true,
        logout_if_target(&client, &OfflineWifi),
        async {
            deleted.set(true);
            Ok(())
        },
        async { Ok(()) },
    )
    .await?;
    assert!(deleted.get());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}
