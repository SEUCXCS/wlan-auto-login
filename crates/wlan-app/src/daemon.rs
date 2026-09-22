use crate::{
    config::{Paths, Preferences},
    control, credentials,
    platform::Monitor,
    shutdown::{Shutdown, until_stopped},
};
use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::{AuthState, Client};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(30);
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Policy {
    initialized: bool,
    session: Option<String>,
    pending_connection: bool,
    credential_blocked: bool,
    revision: u64,
    failures: u32,
    retry_at: Option<Instant>,
}

impl Policy {
    fn update(&mut self, session: Option<String>, control: &mut control::State) -> bool {
        let changed = self.session != session;
        let saved_changed = control.session != session;
        if saved_changed {
            // Native notification generations restart with the monitor. Preserve
            // a manual pause on the same interface during a daemon restart.
            let same_interface_on_restart = !self.initialized
                && control
                    .session
                    .as_deref()
                    .zip(session.as_deref())
                    .is_some_and(|(old, new)| old.split('#').next() == new.split('#').next());
            if !same_interface_on_restart {
                control.paused = false;
            }
            control.session = session.clone();
        }
        self.initialized = true;
        if changed {
            self.session = session.clone();
            self.pending_connection = session.is_some();
            self.failures = 0;
            self.retry_at = None;
        }
        if self.revision != control.revision {
            self.revision = control.revision;
            self.credential_blocked = false;
            self.failures = 0;
            self.retry_at = None;
        }
        changed || saved_changed
    }
    fn should_check(&self, control: &control::State, now: Instant) -> bool {
        self.session.is_some()
            && !control.paused
            && !self.credential_blocked
            && self.retry_at.is_none_or(|at| now >= at)
    }
    fn should_login(&self, auth: AuthState) -> bool {
        auth == AuthState::Offline || (self.pending_connection && auth == AuthState::Unknown)
    }
    fn success(&mut self) {
        self.pending_connection = false;
        self.failures = 0;
        self.retry_at = None;
    }
    fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let seconds = 30u64
            .saturating_mul(1u64 << self.failures.saturating_sub(1).min(4))
            .min(300);
        self.retry_at = Some(now + Duration::from_secs(seconds));
    }
}

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
                    policy.initialized = false;
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
    let previous_revision = policy.revision;
    let changed = policy.update(session, &mut state);
    if changed {
        state.save(paths)?;
    }
    if !scheduled && !changed && policy.revision == previous_revision {
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
    if monitor.snapshot().await? != policy.session {
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
            policy.credential_blocked = true;
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
    #[test]
    fn disconnected_paused_and_invalid_credentials_never_login() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        let now = Instant::now();
        assert!(!policy.should_check(&state, now));
        policy.update(Some("wifi-1".into()), &mut state);
        assert!(policy.should_check(&state, now));
        state.paused = true;
        assert!(!policy.should_check(&state, now));
        policy.update(Some("wifi-2".into()), &mut state);
        assert!(!state.paused);
        policy.credential_blocked = true;
        assert!(!policy.should_check(&state, now));
        state.resume();
        policy.update(Some("wifi-2".into()), &mut state);
        assert!(policy.should_check(&state, now));
    }
    #[test]
    fn unknown_status_only_allows_initial_connection_login() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        assert!(policy.should_login(AuthState::Unknown));
        assert!(!policy.should_login(AuthState::Online));
        policy.success();
        assert!(!policy.should_login(AuthState::Unknown));
        assert!(policy.should_login(AuthState::Offline));
    }
    #[test]
    fn retry_backoff_is_bounded_and_cleared_on_reconnection() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        let now = Instant::now();
        for _ in 0..20 {
            policy.failure(now);
        }
        assert!(!policy.should_check(&state, now + Duration::from_secs(299)));
        assert!(policy.should_check(&state, now + Duration::from_secs(300)));
        policy.update(Some("wifi-2".into()), &mut state);
        assert!(policy.should_check(&state, now));
    }
    #[test]
    fn native_monitor_restart_does_not_clear_manual_pause() {
        let mut state = control::State {
            paused: true,
            session: Some("interface#7".into()),
            revision: 0,
        };
        let mut policy = Policy::default();
        policy.update(Some("interface#0".into()), &mut state);
        assert!(state.paused);
        policy.update(None, &mut state);
        policy.update(Some("interface#1".into()), &mut state);
        assert!(!state.paused);
    }

    #[test]
    fn successful_check_does_not_delay_the_next_scheduled_tick() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        let scheduled = Instant::now() + INTERVAL;
        policy.success();
        assert!(policy.should_check(&state, scheduled));
    }

    #[test]
    fn restart_on_same_connection_preserves_manual_logout() {
        let mut state = control::State {
            paused: true,
            session: Some("wifi-1".into()),
            revision: 0,
        };
        let mut policy = Policy::default();
        policy.update(Some("wifi-1".into()), &mut state);
        assert!(!policy.should_check(&state, Instant::now()));
    }
}
