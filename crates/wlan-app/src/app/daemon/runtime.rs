//! Schedule checks against deadlines and process Wi-Fi and IPC events.
use super::{check::tick, credentials, ensure_scope, policy::Policy};
use crate::infra::{
    control,
    error::{Context, Message, Result, bail},
    paths::Paths,
};
use crate::system::wifi::Monitor;
use cxxy_wlanlogin_lib::Client;
use std::time::Duration;
use tokio::time::Instant;
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) async fn control_stop(
    commands: &mut tokio::sync::mpsc::Receiver<control::Signal>,
    wake: &tokio::sync::Notify,
) -> Result<()> {
    while let Some(command) = commands.recv().await {
        match command {
            control::Signal::Stop => return Ok(()),
            control::Signal::Wake => wake.notify_one(),
        }
    }
    bail!(Message::LocalControlChannelClosed)
}
pub(super) async fn run_loop(
    client: &Client,
    paths: &Paths,
    wake: &tokio::sync::Notify,
) -> Result<()> {
    let mut policy = Policy::default();
    let mut store = credentials::BackgroundReader::new(paths)?;
    // NoEntry is different from a locked/unavailable keyring: the latter may
    // become available after the desktop session finishes unlocking.
    match store.load().await {
        Ok(None) => {
            bail!(Message::DaemonCredentialsMissing)
        }
        Ok(Some(_)) => {}
        Err(e) => tracing::warn!(error = %e, "{}", Message::CredentialsUnavailable),
    }
    let mut monitor = None;
    let mut scheduled = true;
    tracing::info!(
        scope = paths.scope.label(),
        ssid = "Student_CX",
        "{}",
        Message::DaemonStarted
    );
    loop {
        ensure_scope(paths)?;
        if monitor.is_none() {
            match Monitor::new().await {
                Ok(value) => {
                    monitor = Some(value);
                    policy.monitor_restarted();
                }
                Err(e) => {
                    tracing::warn!(error = %e, retry_after_secs = 30, "{}", Message::MonitorUnavailable)
                }
            }
        }
        if let Some(monitor) = &monitor
            && let Err(e) = tokio::time::timeout(
                CHECK_TIMEOUT,
                tick(client, paths, monitor, store.load(), &mut policy, scheduled),
            )
            .await
            .context(Message::NetworkCheckTimedOut)
            .and_then(|result| result)
        {
            let transient_status = e
                .downcast_ref::<cxxy_wlanlogin_lib::Error>()
                .is_some_and(|error| error.is_transient_status_failure());
            let delay = policy.failure(Instant::now(), transient_status);
            tracing::warn!(error = %e, retry_after_secs = delay.as_secs(), "{}", Message::CheckFailed);
        }
        tokio::select! {
            _ = next_scheduled_check(&mut policy) => { scheduled = true; },
            _ = wake.notified() => { scheduled = true; },
            event = async { match &mut monitor { Some(m) => m.changed().await, None => std::future::pending().await } } => {
                if let Err(e) = event { tracing::warn!(error = %e, "{}", Message::MonitorDisconnected); monitor = None; }
                scheduled = false;
                // Merge bursts of state/DHCP notifications.
                tokio::time::sleep(Duration::from_millis(400)).await;
            },
        }
    }
}
pub(super) async fn next_scheduled_check(policy: &mut Policy) {
    tokio::time::sleep_until(policy.next_deadline(Instant::now())).await;
    // Consume even skipped checks so a pause cannot leave a ready timer spinning.
    policy.clear_due_check(Instant::now());
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::shutdown::until_stopped;
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
