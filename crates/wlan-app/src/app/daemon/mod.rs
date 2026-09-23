//! Run the automatic-login loop and coordinate its event sources.
mod policy;
use policy::Policy;

use crate::{
    config::{Paths, Preferences},
    control, credentials,
    shutdown::{Shutdown, until_stopped},
    wifi::{Monitor, WifiConnection},
};
use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::{AuthState, Client, Credentials, LoginOutcome};
use std::{future::Future, time::Duration};
use tokio::time::Instant;

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
                tick(client, paths, monitor, store.load(), &mut policy, scheduled),
            )
            .await
            .context("本轮网络检查超时")
            .and_then(|result| result)
        {
            let transient_status = e
                .downcast_ref::<cxxy_wlanlogin_lib::Error>()
                .is_some_and(|error| error.is_transient_status_failure());
            let delay = policy.failure(Instant::now(), transient_status);
            tracing::warn!("本次检查未完成，{} 秒后重试：{e:#}", delay.as_secs());
        }
        tokio::select! {
            _ = next_scheduled_check(&mut policy) => { scheduled = true; },
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

async fn next_scheduled_check(policy: &mut Policy) {
    tokio::time::sleep_until(policy.next_deadline(Instant::now())).await;
    // Consume even skipped checks so a pause cannot leave a ready timer spinning.
    policy.clear_due_check(Instant::now());
}

async fn tick(
    client: &Client,
    paths: &Paths,
    monitor: &impl WifiConnection,
    read_credentials: impl Future<Output = Result<Option<Credentials>>>,
    policy: &mut Policy,
    scheduled: bool,
) -> Result<()> {
    let session = monitor.target_session().await?;
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
    let status = client.status().await?;
    policy.status_received();
    if status.auth == AuthState::Online {
        policy.online();
        return Ok(());
    }
    if !policy.should_login(status.auth) {
        tracing::debug!("门户未提供可识别的认证状态，本轮不提交登录");
        return Ok(());
    }
    let saved = read_credentials
        .await?
        .ok_or_else(|| anyhow::anyhow!("保存的凭据已不存在，请重新执行 login --remember-me"))?;
    // Recheck the Wi-Fi immediately before sending credentials after an await.
    if monitor.target_session().await?.as_deref() != policy.session() {
        return Ok(());
    }
    let line = Preferences::load(paths)?.line;
    match client.login(&saved, line).await {
        Ok(LoginOutcome::Authenticated) => {
            policy.success();
            tracing::info!("校园网登录成功");
        }
        Ok(LoginOutcome::AlreadyOnline) => policy.online(),
        Err(error) if error.is_credential_rejected() => {
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
        for (responses, reads_secret, seconds, blocked) in [
            (vec![ONLINE], false, 240, false),
            (vec![UNKNOWN], false, 30, false),
            (vec![OFFLINE, ONLINE], true, 240, false),
            (
                vec![OFFLINE, OFFLINE, r#"dr1({"result":1});"#],
                true,
                30,
                false,
            ),
            (
                vec![OFFLINE, OFFLINE, r#"dr1({"result":0,"ret_code":1});"#],
                true,
                30,
                true,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let client = Client::new(cxxy_wlanlogin_lib::PortalConfig {
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
                        anyhow::ensure!(request.len() < 16_384, "oversized mock request");
                    }
                    requests.push(String::from_utf8(request)?);
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(30), listener.accept())
                        .await
                        .is_err()
                );
                Ok::<_, anyhow::Error>(requests)
            });
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let mut policy = Policy::default();
            let read = std::cell::Cell::new(false);
            tick(
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

    #[tokio::test(start_paused = true)]
    async fn retry_uses_its_deadline_and_paused_checks_do_not_spin() {
        let start = Instant::now();
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        tokio::time::advance(Duration::from_secs(6)).await;
        policy.failure(Instant::now(), false);
        state.paused = true;
        next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(36));
        assert!(!policy.should_check(&state, Instant::now()));
        assert_eq!(policy.retry_deadline(), None);
        next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(66));
    }

    #[tokio::test(start_paused = true)]
    async fn fast_retries_wake_at_one_two_and_four_seconds_then_back_off() {
        let start = Instant::now();
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        for elapsed in [1, 3, 7, 37, 97, 217, 337] {
            policy.failure(Instant::now(), true);
            next_scheduled_check(&mut policy).await;
            assert_eq!(Instant::now() - start, Duration::from_secs(elapsed));
            assert!(policy.should_check(&state, Instant::now()));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn online_poll_waits_four_minutes_without_thirty_second_wakeups() {
        let start = Instant::now();
        let mut policy = Policy::default();
        policy.success();
        next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(30));
        policy.online();
        next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(270));
        // A transient failure after a healthy period resumes the short backoff.
        assert_eq!(
            policy.failure(Instant::now(), true),
            Duration::from_secs(30)
        );
        next_scheduled_check(&mut policy).await;
        assert_eq!(Instant::now() - start, Duration::from_secs(300));
    }

    #[tokio::test(start_paused = true)]
    async fn wake_keeps_poll_and_retry_deadlines_and_stop_cancels_waiting() -> Result<()> {
        let start = Instant::now();
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        policy.online();
        for elapsed in [10, 20, 30] {
            tokio::time::advance(Duration::from_secs(10)).await;
            let wake = tokio::sync::Notify::new();
            wake.notify_one();
            tokio::select! {
                _ = next_scheduled_check(&mut policy) => panic!("poll is not due"),
                _ = wake.notified() => {},
            }
            assert_eq!(Instant::now() - start, Duration::from_secs(elapsed));
            assert_eq!(
                policy.next_deadline(Instant::now()),
                start + Duration::from_secs(240)
            );
            assert!(!policy.should_check(&state, Instant::now()));
        }
        policy.failure(Instant::now(), false);
        let due = policy.retry_deadline();
        until_stopped(
            async {
                next_scheduled_check(&mut policy).await;
                bail!("retry ran before stop")
            },
            async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(())
            },
        )
        .await?;
        assert_eq!(policy.retry_deadline(), due);
        assert_eq!(Instant::now() - start, Duration::from_millis(30200));
        Ok(())
    }

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
