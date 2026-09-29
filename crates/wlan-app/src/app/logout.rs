//! Pause authentication, optionally stop the daemon and forget local credentials.
use crate::{
    credentials,
    infra::control,
    infra::paths::Paths,
    service,
    wifi::{WifiConnection, WifiProbe},
};
use anyhow::{Context, Result};
use cxxy_wlanlogin_lib::Client;
use std::future::Future;

pub(super) async fn run(client: &Client, paths: &Paths, forget: bool) -> Result<()> {
    if !forget {
        service::ensure_no_conflicting_install(paths)?;
    }
    run_with(
        paths,
        forget,
        async {
            if forget {
                logout_if_target(client, &WifiProbe::new().await).await
            } else {
                client.logout().await.map_err(Into::into)
            }
        },
        credentials::delete(paths),
        async {
            control::stop(paths).await;
            control::wait_stopped(paths).await
        },
    )
    .await
}

async fn logout_if_target(client: &Client, wifi: &impl WifiConnection) -> Result<()> {
    match wifi
        .target_session()
        .await
        .context("无法确认 Wi-Fi，未请求门户注销")?
    {
        Some(_) => client.logout().await.map_err(Into::into),
        None => {
            tracing::info!("当前未连接 Student_CX，仅清除本地登录凭据，未请求门户注销");
            Ok(())
        }
    }
}

async fn run_with(
    paths: &Paths,
    forget: bool,
    logout: impl Future<Output = Result<()>>,
    delete: impl Future<Output = Result<()>>,
    stop: impl Future<Output = Result<()>>,
) -> Result<()> {
    let _service = control::service_lock(paths)?;
    let _operation = control::operation_lock(paths).await?;
    let previous = control::State::load(paths)?;
    let mut paused = previous.clone();
    paused.paused = true;
    paused.save(paths)?;
    if forget {
        // A remote failure must not prevent deleting local secrets or restore
        // automatic login. Attempt all steps and report any incomplete work.
        let mut errors = Vec::new();
        if let Err(error) = stop.await {
            errors.push(format!("停止后台进程失败：{error:#}"));
        }
        if let Err(error) = logout.await {
            errors.push(format!("门户注销未完成：{error:#}"));
        }
        match delete.await {
            Ok(()) => tracing::info!("所选范围的已保存凭据已删除"),
            Err(error) => errors.push(format!("删除凭据失败：{error:#}")),
        }
        anyhow::ensure!(
            errors.is_empty(),
            "完全登出未完成，自动登录保持暂停：{}",
            errors.join("；")
        );
        tracing::info!("本地登录信息已清除，后台自动登录已停止");
    } else {
        if let Err(error) = logout.await {
            previous.save(paths)?;
            return Err(error);
        }
        control::wake(paths).await;
        tracing::info!("已注销");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, fs};

    #[tokio::test]
    async fn forget_removes_local_credentials_even_when_remote_logout_and_stop_fail() -> Result<()>
    {
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
    async fn ordinary_logout_preserves_credentials_and_restores_state_on_remote_failure()
    -> Result<()> {
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
}
