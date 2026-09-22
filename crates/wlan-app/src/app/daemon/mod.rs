//! Run the automatic-login loop and coordinate its event sources.
mod policy;
use policy::Policy;

use crate::{
    config::{Paths, Preferences},
    control, credentials,
    shutdown::{Shutdown, until_stopped},
    wifi::Monitor,
};
use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::{AuthState, Client};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(30);
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);

fn ensure_scope(paths: &Paths) -> Result<()> {
    // User daemons yield when a system installation exists. A boot service must
    // not inspect user homes (which systemd intentionally hides from it).
    if paths.scope == crate::scope::Scope::User {
        crate::service::ensure_no_conflicting_install(paths)?;
    }
    Ok(())
}

pub async fn run(client: &Client, paths: &Paths) -> Result<()> {
    let mut shutdown = Shutdown::new()?;
    until_stopped(run_inner(client, paths), shutdown.wait()).await
}

async fn run_inner(client: &Client, paths: &Paths) -> Result<()> {
    ensure_scope(paths)?;
    credentials::prepare_daemon()?;
    let _singleton = control::singleton(paths)?;
    let (_server, mut commands) = control::serve(paths).await?;
    let wake = tokio::sync::Notify::new();
    until_stopped(
        run_loop(client, paths, &wake),
        control_stop(&mut commands, &wake),
    )
    .await
}

async fn control_stop(
    commands: &mut tokio::sync::mpsc::Receiver<control::Signal>,
    wake: &tokio::sync::Notify,
) -> Result<()> {
    while let Some(command) = commands.recv().await {
        match command {
            control::Signal::Stop => return Ok(()),
            control::Signal::Wake => wake.notify_one(),
        }
    }
    bail!("本地控制通道已关闭")
}

async fn run_loop(client: &Client, paths: &Paths, wake: &tokio::sync::Notify) -> Result<()> {
    let mut policy = Policy::default();
    let mut store = credentials::BackgroundReader::new(paths)?;
    // NoEntry is different from a locked/unavailable keyring: the latter may
    // become available after the desktop session finishes unlocking.
    match store.load().await {
        Ok(None) => bail!("请先执行 login --remember-me 保存账号和密码，再启动 daemon"),
        Ok(Some(_)) => {}
        Err(e) => tracing::warn!("暂时无法读取密钥，等待解锁：{e:#}"),
    }
    let mut monitor = None;
    let mut scheduled = true;
    let mut interval = tokio::time::interval(INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    tracing::info!("daemon 已启动，仅在 Student_CX 上检查校园网状态");
    loop {
        ensure_scope(paths)?;
        if monitor.is_none() {
            match Monitor::new().await {
                Ok(value) => {
                    monitor = Some(value);
                    policy.monitor_restarted();
                }
                Err(e) => tracing::warn!("网络监听暂不可用，30 秒后重试：{e:#}"),
            }
        }
        if let Some(monitor) = &monitor
            && let Err(e) = tokio::time::timeout(
                CHECK_TIMEOUT,
                tick(client, paths, monitor, &mut store, &mut policy, scheduled),
            )
            .await
            .context("本轮网络检查超时")
            .and_then(|result| result)
        {
            policy.failure(Instant::now());
            tracing::warn!("本次检查未完成，稍后重试：{e:#}");
        }
        tokio::select! {
            _ = interval.tick() => { scheduled = true; },
            _ = wake.notified() => { scheduled = true; },
            event = async { match &mut monitor { Some(m) => m.changed().await, None => std::future::pending().await } } => {
                if let Err(e) = event { tracing::warn!("网络事件订阅断开：{e:#}"); monitor = None; }
                scheduled = false;
                // Merge bursts of state/DHCP notifications.
                tokio::time::sleep(Duration::from_millis(400)).await;
            },
        }
    }
}

async fn tick(
    client: &Client,
    paths: &Paths,
    monitor: &Monitor,
    store: &mut credentials::BackgroundReader,
    policy: &mut Policy,
    scheduled: bool,
) -> Result<()> {
    let session = monitor.snapshot().await?;
    let _operation = control::operation_lock(paths).await?;
    let mut state = control::State::load(paths)?;
    let previous_revision = policy.revision();
    let changed = policy.update(session, &mut state);
    if changed {
        state.save(paths)?;
    }
    if !scheduled && !changed && policy.revision() == previous_revision {
        return Ok(());
    }
    if !policy.should_check(&state, Instant::now()) {
        return Ok(());
    }
    let saved = store
        .load()
        .await?
        .ok_or_else(|| anyhow::anyhow!("保存的凭据已不存在，请重新执行 login --remember-me"))?;
    let status = client.status().await?;
    if status.auth == AuthState::Online {
        policy.success();
        return Ok(());
    }
    if !policy.should_login(status.auth) {
        tracing::debug!("门户未提供可识别的认证状态，本轮不提交登录");
        return Ok(());
    }
    // Recheck the Wi-Fi immediately before sending credentials after an await.
    if monitor.snapshot().await?.as_deref() != policy.session() {
        return Ok(());
    }
    let line = Preferences::load(paths)?.line;
    match client.login(&saved, line).await {
        Ok(()) => {
            policy.success();
            tracing::info!("校园网登录成功");
        }
        Err(cxxy_wlanlogin_lib::Error::Rejected {
            credential_rejected: true,
            ..
        }) => {
            policy.reject_credentials();
            tracing::error!(
                "账号或密码被拒绝；暂停自动重试，请执行 login --interactive --remember-me 更新凭据"
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ipc_stop_cancels_busy_check_even_after_wake_burst() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let (_server, mut commands) = control::serve(&paths).await?;
        let wake = tokio::sync::Notify::new();
        let ready = tokio::sync::Notify::new();
        let work = async {
            let _lock = control::operation_lock(&paths).await?;
            ready.notify_one();
            std::future::pending::<Result<()>>().await
        };
        let send_stop = async {
            ready.notified().await;
            for _ in 0..8 {
                control::wake(&paths).await;
            }
            control::stop(&paths).await;
        };
        let (result, ()) = tokio::join!(
            tokio::time::timeout(
                Duration::from_secs(2),
                until_stopped(work, control_stop(&mut commands, &wake))
            ),
            send_stop,
        );
        result??;
        // Cancellation releases the login/logout lock, not just the wait future.
        let _lock =
            tokio::time::timeout(Duration::from_millis(200), control::operation_lock(&paths))
                .await??;
        Ok(())
    }
}
