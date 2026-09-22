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
    let scheduled = Instant::now() + Duration::from_secs(30);
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
