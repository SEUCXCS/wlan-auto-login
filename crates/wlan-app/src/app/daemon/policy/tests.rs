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
