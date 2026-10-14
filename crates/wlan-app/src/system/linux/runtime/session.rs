//! Activation eligibility is checked live, never from the daemon's startup environment.
use crate::infra::error::{Message, Result, ensure};
use std::collections::HashMap;
use zbus::{Connection, Proxy, proxy::CacheProperties, zvariant::OwnedObjectPath};

use crate::system::define::credentials::{SessionIdentity, SessionPermit as Permit, SessionState};

struct Connections {
    user: Connection,
    system: Connection,
    uid: u32,
}
impl Connections {
    async fn native() -> Result<Self> {
        Ok(Self {
            user: Connection::session().await?,
            system: Connection::system().await?,
            uid: unsafe { libc::geteuid() },
        })
    }
}

const LOGIN: &str = "org.freedesktop.login1";
const SYSTEMD: &str = "org.freedesktop.systemd1";

async fn proxy<'a>(
    bus: &'a Connection,
    dest: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<Proxy<'a>> {
    Ok(zbus::proxy::Builder::new(bus)
        .destination(dest)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}

/// A hint is memory-only and must still name an active local graphical session.
/// Logout, a stale hint, or a restarted bus can never grant activation eligibility.
async fn ready_session(hint: Option<&str>) -> Result<Option<SessionIdentity>> {
    ready_session_on(hint, &Connections::native().await?).await
}
async fn ready_session_on(
    hint: Option<&str>,
    connections: &Connections,
) -> Result<Option<SessionIdentity>> {
    let session = &connections.user;
    let system = &connections.system;
    let manager = proxy(
        session,
        SYSTEMD,
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await?;
    let environment: Vec<String> = manager.get_property("Environment").await?;
    let environment: HashMap<String, String> = environment
        .into_iter()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(key, value)| (key.into(), value.into()))
        })
        .collect();
    let target: std::result::Result<OwnedObjectPath, _> = manager
        .call("GetUnit", &("graphical-session.target",))
        .await;
    let active = match target {
        Ok(target) => {
            proxy(
                session,
                SYSTEMD,
                target.as_str(),
                "org.freedesktop.systemd1.Unit",
            )
            .await?
            .get_property::<String>("ActiveState")
            .await?
        }
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
        {
            String::new()
        }
        Err(error) => return Err(error.into()),
    };
    if !eligible_environment(&environment)
        || (hint.is_none_or(|h| h.starts_with('!')) && active != "active")
    {
        return Ok(None);
    }

    // A display variable alone is not proof of a live login: confirm with logind.
    let login = proxy(
        system,
        LOGIN,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
        login.call("ListSessions", &()).await?;
    let uid = connections.uid;
    for (id, user, _, _, path) in sessions {
        if user != uid || !hint_matches(hint, &id) {
            continue;
        }
        let session = proxy(
            system,
            LOGIN,
            path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .await?;
        let kind: String = session.get_property("Type").await?;
        let active: bool = session.get_property("Active").await?;
        let remote: bool = session.get_property("Remote").await?;
        let state: String = session.get_property("State").await?;
        if graphical_session(&kind, active, remote, &state)
            || (hint.is_some_and(|h| !h.starts_with('!'))
                && kind == "tty"
                && active
                && !remote
                && state == "active")
        {
            let started_at: u64 = session.get_property("TimestampMonotonic").await?;
            return Ok(Some(SessionIdentity { id, started_at }));
        }
    }
    Ok(None)
}

fn hint_matches(hint: Option<&str>, id: &str) -> bool {
    hint.is_none_or(|hint| match hint.strip_prefix('!') {
        Some(ended) => !ended.is_empty() && ended != id,
        None => hint == id,
    })
}

fn graphical_session(kind: &str, active: bool, remote: bool, state: &str) -> bool {
    matches!(kind, "wayland" | "x11") && active && !remote && state == "active"
}

fn eligible_environment(environment: &HashMap<String, String>) -> bool {
    // Read from systemd's activation environment, not the early daemon process.
    let wayland = environment.get("WAYLAND_DISPLAY").filter(|s| !s.is_empty());
    let display = environment.get("DISPLAY").filter(|s| !s.is_empty());
    wayland.is_some_and(|name| {
        let path = std::path::Path::new(name);
        if path.is_absolute() {
            path.is_socket()
        } else {
            environment
                .get("XDG_RUNTIME_DIR")
                .is_some_and(|dir| std::path::Path::new(dir).join(path).is_socket())
        }
    }) || display.is_some_and(|name| {
        // Local X11 only. Remote/headless providers remain readable if already owned.
        name.strip_prefix(':')
            .and_then(|s| s.split('.').next())
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .is_some_and(|n| {
                std::path::Path::new("/tmp/.X11-unix")
                    .join(format!("X{n}"))
                    .is_socket()
            })
    })
}
use std::os::unix::fs::FileTypeExt;
trait SocketPath {
    fn is_socket(&self) -> bool;
}
impl SocketPath for std::path::Path {
    fn is_socket(&self) -> bool {
        std::fs::metadata(self).is_ok_and(|m| m.file_type().is_socket())
    }
}

/// Authenticate the hook's process membership instead of trusting XDG_SESSION_ID.
pub async fn current_session_id() -> Result<String> {
    let system = Connection::system().await?;
    let manager = proxy(
        &system,
        LOGIN,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let path: OwnedObjectPath = manager
        .call("GetSessionByPID", &(std::process::id(),))
        .await?;
    let session = proxy(
        &system,
        LOGIN,
        path.as_str(),
        "org.freedesktop.login1.Session",
    )
    .await?;
    let id: String = session.get_property("Id").await?;
    Ok(id)
}

/// Synchronize only display/session variables supplied by the logged-in hook.
/// This updates the actual activation environment; changing our own env is insufficient.
pub async fn import_current_session() -> Result<String> {
    let id = current_session_id().await?;
    let session = Connection::session().await?;
    let mut vars: HashMap<String, String> = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_ID",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
    .collect();
    vars.insert("XDG_SESSION_ID".into(), id.clone());
    ensure!(eligible_environment(&vars), Message::SessionNotReady);
    let bus = proxy(
        &session,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;
    bus.call::<_, _, ()>("UpdateActivationEnvironment", &vars)
        .await?;
    let manager = proxy(
        &session,
        SYSTEMD,
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await?;
    let values: Vec<String> = vars
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    manager
        .call::<_, _, ()>("SetEnvironment", &(values,))
        .await?;
    ensure!(
        ready_session(Some(&id)).await?.is_some(),
        Message::SessionNotReady
    );
    Ok(id)
}

/// Prepare the real activation environment before any standard Secret Service call.
pub async fn prepare_read(
    readiness: &tokio::sync::watch::Receiver<SessionState>,
) -> Result<Option<Permit>> {
    prepare_read_on(readiness, &Connections::native().await?).await
}
async fn prepare_read_on(
    readiness: &tokio::sync::watch::Receiver<SessionState>,
    connections: &Connections,
) -> Result<Option<Permit>> {
    let state = readiness.borrow().clone();
    let Some(identity) = ready_session_on(state.hint.as_deref(), connections).await? else {
        return Ok(None);
    };

    // Synchronize the actual bus environment after confirming a live session.
    let session = &connections.user;
    let manager = proxy(
        session,
        SYSTEMD,
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await?;
    let values: Vec<String> = manager.get_property("Environment").await?;
    let source: HashMap<String, String> = values
        .into_iter()
        .filter_map(|v| v.split_once('=').map(|(k, v)| (k.into(), v.into())))
        .collect();
    let mut environment: HashMap<String, String> = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
    ]
    .into_iter()
    .map(|key| (key.into(), source.get(key).cloned().unwrap_or_default()))
    .collect();
    environment.insert("XDG_SESSION_ID".into(), identity.id.clone());
    proxy(
        session,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?
    .call::<_, _, ()>("UpdateActivationEnvironment", &environment)
    .await?;

    // Preserve identity and generation in the result, including across a cached task.
    let permit = Permit { identity, state };
    if valid_on(&permit, readiness, connections).await? {
        Ok(Some(permit))
    } else {
        Ok(None)
    }
}

/// Every consumer checkpoint queries live session state without reading credentials.
pub async fn valid(
    permit: &Permit,
    readiness: &tokio::sync::watch::Receiver<SessionState>,
) -> Result<bool> {
    valid_on(permit, readiness, &Connections::native().await?).await
}
async fn valid_on(
    permit: &Permit,
    readiness: &tokio::sync::watch::Receiver<SessionState>,
    connections: &Connections,
) -> Result<bool> {
    let before = readiness.borrow().clone();
    if before != permit.state {
        return Ok(false);
    }
    let current = ready_session_on(permit.state.hint.as_deref(), connections).await?;
    let after = readiness.borrow().clone();
    Ok(after == permit.state && current.as_ref() == Some(&permit.identity))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logout_and_new_login_do_not_reuse_readiness_from_an_old_session() {
        assert!(hint_matches(Some("old"), "old"));
        assert!(!hint_matches(Some("old"), "new"));
        assert!(!hint_matches(Some("!old"), "old"));
        assert!(hint_matches(Some("!old"), "new"));
        assert!(hint_matches(None, "new"));
    }
    #[test]
    fn display_alone_and_dead_sessions_never_authorize_activation() {
        assert!(!eligible_environment(&HashMap::from([(
            "DISPLAY".into(),
            ":987654".into()
        )])));
        for (kind, active, remote, state) in [
            ("tty", true, false, "active"),
            ("wayland", false, false, "online"),
            ("x11", true, true, "active"),
            ("wayland", true, false, "closing"),
        ] {
            assert!(!graphical_session(kind, active, remote, state));
        }
        assert!(graphical_session("wayland", true, false, "active"));
    }

    use crate::infra::error::Context;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Stdio},
        sync::{Arc, Mutex},
    };

    fn desktop_state(
        state: &Arc<Mutex<Desktop>>,
    ) -> zbus::fdo::Result<std::sync::MutexGuard<'_, Desktop>> {
        state
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("mock state lock poisoned".into()))
    }
    fn object_path(path: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(path).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
    type Sessions = Vec<(String, u32, String, String, OwnedObjectPath)>;
    struct QueryGate {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[derive(Clone)]
    struct Desktop {
        environment: Vec<String>,
        target: String,
        id: String,
        uid: u32,
        started_at: u64,
        kind: String,
        active: bool,
        remote: bool,
        state: String,
        fail: bool,
        gate: Option<Arc<QueryGate>>,
    }
    struct Systemd(Arc<Mutex<Desktop>>);
    #[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
    impl Systemd {
        #[zbus(property)]
        fn environment(&self) -> zbus::fdo::Result<Vec<String>> {
            Ok(desktop_state(&self.0)?.environment.clone())
        }
        fn get_unit(&self, name: String) -> zbus::fdo::Result<OwnedObjectPath> {
            assert_eq!(name, "graphical-session.target");
            object_path("/graphical")
        }
    }
    struct Unit(Arc<Mutex<Desktop>>);
    #[zbus::interface(name = "org.freedesktop.systemd1.Unit")]
    impl Unit {
        #[zbus(property)]
        fn active_state(&self) -> zbus::fdo::Result<String> {
            Ok(desktop_state(&self.0)?.target.clone())
        }
    }
    struct Logind(Arc<Mutex<Desktop>>);
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Logind {
        async fn list_sessions(&self) -> zbus::fdo::Result<Sessions> {
            let desktop = desktop_state(&self.0)?.clone();
            if let Some(gate) = &desktop.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if desktop.fail {
                return Err(zbus::fdo::Error::Failed("mock query failure".into()));
            }
            Ok(vec![(
                desktop.id.clone(),
                desktop.uid,
                "mock-user".into(),
                "seat0".into(),
                object_path("/session")?,
            )])
        }
    }
    struct LoginSession(Arc<Mutex<Desktop>>);
    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl LoginSession {
        #[zbus(property, name = "Type")]
        fn kind(&self) -> zbus::fdo::Result<String> {
            Ok(desktop_state(&self.0)?.kind.clone())
        }
        #[zbus(property)]
        fn active(&self) -> zbus::fdo::Result<bool> {
            Ok(desktop_state(&self.0)?.active)
        }
        #[zbus(property)]
        fn remote(&self) -> zbus::fdo::Result<bool> {
            Ok(desktop_state(&self.0)?.remote)
        }
        #[zbus(property)]
        fn state(&self) -> zbus::fdo::Result<String> {
            Ok(desktop_state(&self.0)?.state.clone())
        }
        #[zbus(property)]
        fn timestamp_monotonic(&self) -> zbus::fdo::Result<u64> {
            Ok(desktop_state(&self.0)?.started_at)
        }
    }

    struct PrivateDesktop {
        child: Child,
        _directory: tempfile::TempDir,
        _stdout: BufReader<std::process::ChildStdout>,
        _socket: std::os::unix::net::UnixListener,
        _server: Connection,
        connections: Connections,
        state: Arc<Mutex<Desktop>>,
    }
    impl PrivateDesktop {
        async fn start() -> Result<Self> {
            let directory = tempfile::tempdir()?;
            let address = format!(
                "unix:path={}",
                directory.path().join("bus.socket").display()
            );
            let mut child = std::process::Command::new("dbus-daemon")
                .args([
                    "--session",
                    "--nofork",
                    "--print-address=1",
                    "--address",
                    &address,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let mut stdout = BufReader::new(child.stdout.take().context("private daemon stdout")?);
            let mut announced = String::new();
            stdout.read_line(&mut announced)?;
            let address = announced.trim();
            let socket =
                std::os::unix::net::UnixListener::bind(directory.path().join("wayland-mock"))?;
            let state = Arc::new(Mutex::new(Desktop {
                environment: vec![
                    format!("XDG_RUNTIME_DIR={}", directory.path().display()),
                    "WAYLAND_DISPLAY=wayland-mock".into(),
                ],
                target: "active".into(),
                id: "old".into(),
                uid: 1000,
                started_at: 100,
                kind: "wayland".into(),
                active: true,
                remote: false,
                state: "active".into(),
                fail: false,
                gate: None,
            }));
            let server = zbus::connection::Builder::address(address)?
                .name(SYSTEMD)?
                .name(LOGIN)?
                .serve_at("/org/freedesktop/systemd1", Systemd(state.clone()))?
                .serve_at("/graphical", Unit(state.clone()))?
                .serve_at("/org/freedesktop/login1", Logind(state.clone()))?
                .serve_at("/session", LoginSession(state.clone()))?
                .build()
                .await?;
            let client = zbus::connection::Builder::address(address)?.build().await?;
            Ok(Self {
                child,
                _directory: directory,
                _stdout: stdout,
                _socket: socket,
                _server: server,
                connections: Connections {
                    user: client.clone(),
                    system: client,
                    uid: 1000,
                },
                state,
            })
        }
    }
    impl Drop for PrivateDesktop {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[tokio::test]
    async fn real_readiness_queries_reject_unready_or_dead_desktops() -> Result<()> {
        let desktop = PrivateDesktop::start().await?;
        let ready = desktop_state(&desktop.state)
            .context("mock desktop state")?
            .clone();
        let (_, receiver) = tokio::sync::watch::channel(SessionState::default());
        assert!(
            prepare_read_on(&receiver, &desktop.connections)
                .await?
                .is_some()
        );
        for case in 0..7 {
            let mut state = ready.clone();
            match case {
                0 => state.target = "inactive".into(),
                1 => state.environment = vec!["WAYLAND_DISPLAY=missing".into()],
                2 => state.active = false,
                3 => state.remote = true,
                4 => state.state = "closing".into(),
                5 => state.uid = 2000,
                _ => state.kind = "tty".into(),
            }
            *desktop_state(&desktop.state).context("mock desktop state")? = state;
            assert!(
                prepare_read_on(&receiver, &desktop.connections)
                    .await?
                    .is_none()
            );
        }
        *desktop_state(&desktop.state).context("mock desktop state")? = ready;
        desktop_state(&desktop.state)
            .context("mock desktop state")?
            .fail = true;
        assert!(
            prepare_read_on(&receiver, &desktop.connections)
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn live_session_identity_and_generations_reject_logout_relogin_and_aba() -> Result<()> {
        let desktop = PrivateDesktop::start().await?;
        let ready = desktop_state(&desktop.state)
            .context("mock desktop state")?
            .clone();
        let (sender, receiver) = tokio::sync::watch::channel(SessionState::default());
        let permit = prepare_read_on(&receiver, &desktop.connections)
            .await?
            .context("expected permit")?;
        assert!(valid_on(&permit, &receiver, &desktop.connections).await?);
        // No hook hint is needed to detect logout, new IDs, or reuse of the same ID.
        for case in 0..3 {
            let mut state = ready.clone();
            match case {
                0 => state.state = "closing".into(),
                1 => state.id = "new".into(),
                _ => state.started_at += 1,
            }
            *desktop_state(&desktop.state).context("mock desktop state")? = state;
            assert!(!valid_on(&permit, &receiver, &desktop.connections).await?);
        }
        *desktop_state(&desktop.state).context("mock desktop state")? = ready;
        let mut current = receiver.borrow().clone();
        current.update(None)?;
        sender.send_replace(current);
        assert!(!valid_on(&permit, &receiver, &desktop.connections).await?);
        current = receiver.borrow().clone();
        current.update(Some("old".into()))?;
        sender.send_replace(current);
        let permit = prepare_read_on(&receiver, &desktop.connections)
            .await?
            .context("expected hinted permit")?;
        current = receiver.borrow().clone();
        current.update(Some("!old".into()))?;
        current.update(Some("old".into()))?;
        sender.send_replace(current);
        assert!(!valid_on(&permit, &receiver, &desktop.connections).await?);
        let permit = prepare_read_on(&receiver, &desktop.connections)
            .await?
            .context("expected new permit")?;
        desktop_state(&desktop.state)
            .context("mock desktop state")?
            .fail = true;
        assert!(
            valid_on(&permit, &receiver, &desktop.connections)
                .await
                .is_err()
        );
        Ok(())
    }
    #[tokio::test]
    async fn generation_is_rechecked_after_awaited_live_queries() -> Result<()> {
        let desktop = PrivateDesktop::start().await?;
        let (sender, receiver) = tokio::sync::watch::channel(SessionState::default());
        let permit = prepare_read_on(&receiver, &desktop.connections)
            .await?
            .context("expected permit")?;
        let gate = Arc::new(QueryGate {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        desktop_state(&desktop.state)
            .context("mock desktop state")?
            .gate = Some(gate.clone());
        let (result, changed) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(valid_on(&permit, &receiver, &desktop.connections), async {
                gate.entered.notified().await;
                let mut next = receiver.borrow().clone();
                next.update(Some("!old".into()))?;
                next.update(None)?;
                sender.send_replace(next);
                gate.release.notify_one();
                Ok::<_, crate::infra::error::Error>(())
            })
        })
        .await?;
        changed?;
        assert!(!result?);
        Ok(())
    }
}
