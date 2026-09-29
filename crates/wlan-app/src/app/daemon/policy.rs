//! Login decisions and retry state; no network, filesystem or task operations.
use crate::infra::control;
use cxxy_wlanlogin_lib::AuthState;
use std::time::Duration;
use tokio::time::Instant;

const INITIAL_STATUS_DELAYS: [u64; 3] = [1, 2, 4];
const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const ONLINE_INTERVAL: Duration = Duration::from_secs(240);

#[derive(Default)]
pub(super) struct Policy {
    initialized: bool,
    session: Option<String>,
    initial_status_pending: bool,
    initial_status_failures: usize,
    credential_blocked: bool,
    revision: u64,
    failures: u32,
    retry_at: Option<Instant>,
    poll_at: Option<Instant>,
}

impl Policy {
    pub(super) fn monitor_restarted(&mut self) {
        self.initialized = false;
        self.poll_at = None;
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub(super) fn reject_credentials(&mut self) {
        self.credential_blocked = true;
    }

    pub(super) fn update(&mut self, session: Option<String>, control: &mut control::State) -> bool {
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
            self.initial_status_pending = session.is_some();
            self.initial_status_failures = 0;
            self.failures = 0;
            self.retry_at = None;
            self.poll_at = None;
        }
        if self.revision != control.revision {
            self.revision = control.revision;
            self.credential_blocked = false;
            self.initial_status_failures = 0;
            self.failures = 0;
            self.retry_at = None;
            self.poll_at = None;
        }
        changed || saved_changed
    }
    pub(super) fn should_check(&self, control: &control::State, now: Instant) -> bool {
        self.session.is_some()
            && !control.paused
            && !self.credential_blocked
            && self.retry_at.is_none_or(|at| now >= at)
            && self.poll_at.is_none_or(|at| now >= at)
    }
    pub(super) fn should_login(&self, auth: AuthState) -> bool {
        auth == AuthState::Offline
    }
    pub(super) fn success(&mut self) {
        self.status_received();
        self.failures = 0;
        self.retry_at = None;
        // A successful login action gets a prompt status confirmation first.
        self.poll_at = Some(Instant::now() + CHECK_INTERVAL);
    }
    pub(super) fn online(&mut self) {
        self.success();
        self.poll_at = Some(Instant::now() + ONLINE_INTERVAL);
    }
    pub(super) fn status_received(&mut self) {
        self.initial_status_pending = false;
    }

    #[cfg(test)]
    pub(super) fn retry_deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub(super) fn next_deadline(&mut self, now: Instant) -> Instant {
        self.retry_at
            .unwrap_or_else(|| *self.poll_at.get_or_insert(now + CHECK_INTERVAL))
    }

    pub(super) fn clear_due_check(&mut self, now: Instant) {
        if self.retry_at.is_some_and(|at| now >= at) {
            self.retry_at = None;
        }
        if self.poll_at.is_some_and(|at| now >= at) {
            self.poll_at = None;
        }
    }

    pub(super) fn failure(&mut self, now: Instant, transient_status: bool) -> Duration {
        self.poll_at = None;
        if self.initial_status_pending
            && transient_status
            && let Some(&seconds) = INITIAL_STATUS_DELAYS.get(self.initial_status_failures)
        {
            self.initial_status_failures += 1;
            let delay = Duration::from_secs(seconds);
            self.retry_at = Some(now + delay);
            return delay;
        }
        self.failures = self.failures.saturating_add(1);
        let seconds = 30u64
            .saturating_mul(1u64 << self.failures.saturating_sub(1).min(4))
            .min(120);
        let delay = Duration::from_secs(seconds);
        self.retry_at = Some(now + delay);
        delay
    }
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
    fn unknown_status_never_allows_automatic_login_even_on_initial_connection() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        assert!(!policy.should_login(AuthState::Unknown));
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
            policy.failure(now, false);
        }
        assert!(!policy.should_check(&state, now + Duration::from_secs(119)));
        assert!(policy.should_check(&state, now + Duration::from_secs(120)));
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
    fn online_status_defers_polling_but_reconnection_and_explicit_login_check_immediately() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        policy.online();
        let now = Instant::now();
        assert!(!policy.should_check(&state, now + Duration::from_secs(239)));
        assert!(policy.should_check(&state, now + Duration::from_secs(240)));
        policy.update(Some("wifi-2".into()), &mut state);
        assert!(policy.should_check(&state, now));
        policy.online();
        state.resume();
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

    #[test]
    fn initial_status_retries_are_fast_bounded_and_respect_the_deadline() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        let mut now = Instant::now();
        for seconds in [1, 2, 4, 30, 60, 120, 120, 120, 120] {
            let delay = policy.failure(now, true);
            assert_eq!(delay, Duration::from_secs(seconds));
            now += delay;
            assert!(!policy.should_check(&state, now - Duration::from_nanos(1)));
            assert!(policy.should_check(&state, now));
        }

        policy.update(Some("wifi-2".into()), &mut state);
        assert!(policy.should_check(&state, now));
        assert_eq!(policy.failure(now, true), Duration::from_secs(1));
    }

    #[test]
    fn non_status_errors_and_login_failures_use_normal_backoff() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        let now = Instant::now();
        assert_eq!(policy.failure(now, false), Duration::from_secs(30));

        // A valid status response ends the fast window even before login succeeds.
        policy.status_received();
        assert!(!policy.should_login(AuthState::Unknown));
        assert_eq!(policy.failure(now, false), Duration::from_secs(60));
        assert_eq!(policy.failure(now, true), Duration::from_secs(120));
        policy.success();
        assert_eq!(policy.retry_deadline(), None);
        assert_eq!(policy.failure(now, true), Duration::from_secs(30));
    }

    #[test]
    fn a_due_fast_retry_cannot_override_pause_disconnect_or_rejected_credentials() {
        let mut policy = Policy::default();
        let mut state = control::State::default();
        policy.update(Some("wifi-1".into()), &mut state);
        let now = Instant::now();
        let due = now + policy.failure(now, true);
        state.paused = true;
        policy.clear_due_check(due);
        assert!(!policy.should_check(&state, due));

        state.paused = false;
        policy.reject_credentials();
        assert!(!policy.should_check(&state, due));
        policy.update(None, &mut state);
        assert!(!policy.should_check(&state, due));
    }
}
