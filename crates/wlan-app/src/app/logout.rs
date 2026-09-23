//! Pause authentication, optionally stop the daemon and forget local credentials.
use crate::{
    config::Paths,
    control, credentials, service,
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
mod tests;
