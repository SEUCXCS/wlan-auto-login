use crate::{
    config::{Paths, Preferences},
    control, credentials,
    platform::Monitor,
};
use anyhow::{Result, bail};
use cxxy_wlanlogin_lib::{AuthState, Client};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Policy {
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
        if changed {
            self.session = session.clone();
            self.pending_connection = session.is_some();
            self.failures = 0;
            self.retry_at = None;
            if control.session != session {
                control.paused = false;
                control.session = session;
            }
        }
        if self.revision != control.revision {
            self.revision = control.revision;
            self.credential_blocked = false;
            self.failures = 0;
            self.retry_at = None;
        }
        changed
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
        self.retry_at = Some(Instant::now() + INTERVAL);
    }
    fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let seconds = 30u64
            .saturating_mul(1u64 << self.failures.saturating_sub(1).min(4))
            .min(300);
        self.retry_at = Some(now + Duration::from_secs(seconds));
    }
}

pub async fn run(client: &Client, paths: &Paths) -> Result<()> {
    let _singleton = control::singleton(paths)?;
    let (_server, mut wake) = control::serve(paths).await?;
    let mut policy = Policy::default();
    // NoEntry is different from a locked/unavailable keyring: the latter may
    // become available after the desktop session finishes unlocking.
    match credentials::load().await {
        Ok(None) => bail!("请先执行 login --remember-me 保存账号和密码，再启动 daemon"),
        Ok(Some(_)) => {}
        Err(e) => tracing::warn!("暂时无法读取密钥，等待解锁：{e:#}"),
    }
    let mut monitor = None;
    let mut interval = tokio::time::interval(INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    tracing::info!("daemon 已启动，仅在 Student_CX 上检查校园网状态");
    loop {
        if monitor.is_none() {
            match Monitor::new().await {
                Ok(value) => monitor = Some(value),
                Err(e) => tracing::warn!("网络监听暂不可用，30 秒后重试：{e:#}"),
            }
        }
        if let Some(monitor) = &monitor
            && let Err(e) = tick(client, paths, monitor, &mut policy).await
        {
            policy.failure(Instant::now());
            tracing::warn!("本次检查未完成，稍后重试：{e:#}");
        }
        tokio::select! {
            _ = interval.tick() => {},
            _ = wake.recv() => {},
            event = async { match &mut monitor { Some(m) => m.changed().await, None => std::future::pending().await } } => {
                if let Err(e) = event { tracing::warn!("网络事件订阅断开：{e:#}"); monitor = None; }
                // Merge bursts of state/DHCP notifications.
                tokio::time::sleep(Duration::from_millis(400)).await;
            },
            _ = shutdown() => { tracing::info!("daemon 已停止"); return Ok(()); }
        }
    }
}

async fn tick(
    client: &Client,
    paths: &Paths,
    monitor: &Monitor,
    policy: &mut Policy,
) -> Result<()> {
    let session = monitor.snapshot().await?;
    let _operation = control::operation_lock(paths).await?;
    let mut state = control::State::load(paths)?;
    let previous_session = state.session.clone();
    if policy.update(session, &mut state) || previous_session != state.session {
        state.save(paths)?;
    }
    if !policy.should_check(&state, Instant::now()) {
        return Ok(());
    }
    let saved = credentials::load()
        .await?
        .ok_or_else(|| anyhow::anyhow!("保存的凭据已不存在，请重新执行 login --remember-me"))?;
    let status = client.status().await?;
    if status.auth == AuthState::Online {
        policy.success();
        return Ok(());
    }
    if !policy.should_login(status.auth) {
        policy.retry_at = Some(Instant::now() + INTERVAL);
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
            tracing::error!("账号或密码被拒绝；暂停自动重试，请执行 login --remember-me 更新凭据");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
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
