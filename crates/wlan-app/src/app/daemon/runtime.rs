//! Schedule checks against deadlines and process Wi-Fi and IPC events.
use super::{
    check::{Outcome, tick},
    credentials, ensure_scope,
    policy::Policy,
};
use crate::infra::{
    control,
    error::{Context, Message, Result, bail},
    paths::Paths,
};
use std::time::Duration;
use tokio::time::Instant;
use wlanlogin_lib::Client;
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(super) async fn control_stop(
    commands: &mut tokio::sync::mpsc::Receiver<control::Signal>,
    wake: &tokio::sync::Notify,
) -> Result<()> {
    let (session, _) =
        tokio::sync::watch::channel(crate::system::define::credentials::SessionState::default());
    control_stop_with_session(commands, wake, &session).await
}

pub(super) async fn control_stop_with_session(
    commands: &mut tokio::sync::mpsc::Receiver<control::Signal>,
    wake: &tokio::sync::Notify,
    session: &tokio::sync::watch::Sender<crate::system::define::credentials::SessionState>,
) -> Result<()> {
    while let Some(command) = commands.recv().await {
        match command {
            control::Signal::Stop => return Ok(()),
            control::Signal::Wake => wake.notify_one(),
            control::Signal::SessionReady(id) => {
                let mut next = session.borrow().clone();
                next.update(Some(id))?;
                session.send_replace(next);
                wake.notify_one();
            }
            control::Signal::SessionEnd(ended) => {
                let mut next = session.borrow().clone();
                next.update(Some(format!("!{ended}")))?;
                session.send_replace(next);
                wake.notify_one();
            }
        }
    }
    bail!(Message::LocalControlChannelClosed)
}
pub(super) async fn run_loop(
    client: &Client,
    paths: &Paths,
    wake: &tokio::sync::Notify,
    mut session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
    status: &std::sync::RwLock<control::RuntimeStatus>,
) -> Result<()> {
    let mut policy = Policy::default();
    let mut store = credentials::BackgroundReader::new(paths)?;
    let mut credential_events: Option<CredentialEvents> = None;
    let mut monitor = None;
    let mut scheduled = true;
    let mut last_outcome = None;
    tracing::info!(
        scope = paths.scope.label(),
        ssid = "Student_CX",
        "{}",
        Message::DaemonStarted
    );
    loop {
        ensure_scope(paths)?;
        // Session hints remain ephemeral; the native reader validates them live.
        session.borrow_and_update();
        store.session_changed(session.clone());
        #[cfg(target_os = "linux")]
        if paths.scope == crate::infra::scope::Scope::User && credential_events.is_none() {
            // Event subscription is bounded and never requests credential activation.
            credential_events = tokio::time::timeout(
                Duration::from_secs(5),
                crate::system::linux::runtime::credentials::Events::new(),
            )
            .await
            .ok()
            .and_then(std::result::Result::ok);
        }

        if monitor.is_none() {
            match crate::system::flow::wifi::monitor::run().await {
                Ok(value) => {
                    monitor = Some(value);
                    policy.monitor_restarted();
                }
                Err(e) => {
                    tracing::warn!(error = %e, retry_after_secs = 30, "{}", Message::MonitorUnavailable)
                }
            }
        }
        if let Some(monitor) = &monitor {
            let current = session.clone();
            let result = tokio::time::timeout(
                CHECK_TIMEOUT,
                tick(
                    client,
                    paths,
                    monitor,
                    store.load(),
                    move |permit| async move {
                        crate::system::flow::credentials::background::validate(
                            paths, &permit, &current,
                        )
                        .await
                    },
                    &mut policy,
                    scheduled,
                ),
            )
            .await
            .context(Message::NetworkCheckTimedOut)
            .and_then(|result| result);
            match result {
                // Notifications and early wakes must not obscure the last actual
                // result or repeat the same waiting message every poll interval.
                Ok(Outcome::Unchanged | Outcome::WaitingForDeadline) => {}
                Ok(outcome) => {
                    if let Ok(mut status) = status.write() {
                        status.outcome = Some(outcome.label().into());
                        status.credentials = outcome.credentials_state();
                        status.diagnostic = outcome.diagnostic();
                    }

                    if last_outcome != Some(outcome) {
                        let now = Instant::now();
                        let delay = policy.next_deadline(now).saturating_duration_since(now);
                        if let Some(diagnostic) = outcome.diagnostic() {
                            tracing::warn!(kind=?diagnostic.kind,stage=diagnostic.stage.label(),native_code=diagnostic.native_code,bus_code=?diagnostic.bus_code,retry_after_secs=delay.as_secs_f64().ceil() as u64,"{}",diagnostic);
                        } else {
                            tracing::info!(
                                outcome = outcome.label(),
                                next_check_secs = delay.as_secs_f64().ceil() as u64,
                                "{}",
                                Message::NetworkCheckCompleted
                            );
                        }
                    }
                    last_outcome = Some(outcome);
                }
                Err(e) => {
                    if let Ok(mut status) = status.write() {
                        status.outcome = Some("network_check_failed".into());
                        status.credentials = control::CredentialState::NotChecked;
                        status.diagnostic = None;
                    }
                    last_outcome = None;
                    let transient_status = e
                        .downcast_ref::<wlanlogin_lib::Error>()
                        .is_some_and(|error| error.is_transient_status_failure());
                    let delay = policy.failure(Instant::now(), transient_status);
                    tracing::warn!(error = %e, retry_after_secs = delay.as_secs(), "{}", Message::CheckFailed);
                }
            }
        }
        tokio::select! {
            _ = next_scheduled_check(&mut policy) => { scheduled = true; },
            _ = wake.notified() => { scheduled = true; policy.credentials_changed(); },
            _ = session.changed() => { scheduled = true; policy.credentials_changed(); },
            event = credential_change(&mut credential_events) => {
                #[cfg(target_os = "linux")]
                if event.is_err() { credential_events = None; }
                let _ = event;
                policy.credentials_changed();
                scheduled = true;
            },
            event = async { match &mut monitor { Some(m) => m.changed().await, None => std::future::pending().await } } => {
                if let Err(e) = event { tracing::warn!(error = %e, "{}", Message::MonitorDisconnected); monitor = None; }
                scheduled = false;
                // Merge bursts of state/DHCP notifications.
                tokio::time::sleep(Duration::from_millis(400)).await;
            },
        }
    }
}
#[cfg(target_os = "linux")]
type CredentialEvents = crate::system::linux::runtime::credentials::Events;
#[cfg(not(target_os = "linux"))]
type CredentialEvents = ();
async fn credential_change(events: &mut Option<CredentialEvents>) -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(events) = events {
        return events.changed().await;
    }
    let _ = events;
    std::future::pending().await
}

pub(super) async fn next_scheduled_check(policy: &mut Policy) {
    tokio::time::sleep_until(policy.next_deadline(Instant::now())).await;
    if policy.retry_deadline().is_some() {
        tracing::info!("{}", Message::RetryingNetworkCheck);
    }
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
