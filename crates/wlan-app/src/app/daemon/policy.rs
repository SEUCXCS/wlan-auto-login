//! Login decisions and retry state; no network, filesystem or task operations.
use crate::control;
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
mod tests;
