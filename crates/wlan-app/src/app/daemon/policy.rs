//! Login decisions and retry state; no network, filesystem or task operations.
use crate::control;
use cxxy_wlanlogin_lib::AuthState;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Policy {
    initialized: bool,
    session: Option<String>,
    pending_connection: bool,
    credential_blocked: bool,
    revision: u64,
    failures: u32,
    retry_at: Option<Instant>,
}

impl Policy {
    pub(super) fn monitor_restarted(&mut self) {
        self.initialized = false;
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
    pub(super) fn should_check(&self, control: &control::State, now: Instant) -> bool {
        self.session.is_some()
            && !control.paused
            && !self.credential_blocked
            && self.retry_at.is_none_or(|at| now >= at)
    }
    pub(super) fn should_login(&self, auth: AuthState) -> bool {
        auth == AuthState::Offline || (self.pending_connection && auth == AuthState::Unknown)
    }
    pub(super) fn success(&mut self) {
        self.pending_connection = false;
        self.failures = 0;
        self.retry_at = None;
    }
    pub(super) fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let seconds = 30u64
            .saturating_mul(1u64 << self.failures.saturating_sub(1).min(4))
            .min(300);
        self.retry_at = Some(now + Duration::from_secs(seconds));
    }
}

#[cfg(test)]
mod tests;
