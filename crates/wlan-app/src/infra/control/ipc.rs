pub(crate) use super::protocol::{CredentialState, RuntimeStatus};
use super::{
    protocol::{self, Command, Request, Response, VERSION},
    transport,
};
use crate::infra::{
    error::{Message, Result, ensure},
    filesystem::{private_dir, write_private},
    paths::Paths,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    version: u32,
    scope: String,
    address: String,
    token: String,
    pid: u32,
}

pub struct Server {
    task: tokio::task::JoinHandle<()>,
    path: std::path::PathBuf,
    pub status: Arc<RwLock<RuntimeStatus>>,
    #[cfg(unix)]
    socket: std::path::PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_file(&self.path);
        #[cfg(unix)]
        let _ = fs::remove_file(&self.socket);
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Signal {
    Wake,
    Stop,
    SessionReady(String),
    SessionEnd(String),
}
#[cfg(test)]
pub async fn serve(paths: &Paths) -> Result<(Server, mpsc::Receiver<Signal>)> {
    serve_with_mode(paths, false).await
}

pub async fn serve_with_mode(
    paths: &Paths,
    managed: bool,
) -> Result<(Server, mpsc::Receiver<Signal>)> {
    // Publish an endpoint only after binding a scope-restricted transport.
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|e| {
        crate::infra::error::error!(Message::FailedToGenerateControlTokenE { e: e.to_string() })
    })?;
    let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
    private_dir(&paths.runtime)?;
    let (mut listener, address) = transport::Listener::bind(paths, &token)?;
    let scope = paths.scope.label().to_string();
    let endpoint = Endpoint {
        version: VERSION,
        scope: scope.clone(),
        address,
        token: token.clone(),
        pid: std::process::id(),
    };
    let path = paths.runtime.join("daemon-endpoint.json");
    write_private(&path, &serde_json::to_vec(&endpoint)?)?;
    let (sender, receiver) = mpsc::channel(16);
    let status = Arc::new(RwLock::new(RuntimeStatus::default()));
    let snapshot = status.clone();
    let task = tokio::spawn(async move {
        while let Ok(mut socket) = listener.accept().await {
            // Dead/malformed clients cannot hold up shutdown indefinitely.
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                let request: Request = protocol::read(&mut socket).await?;
                ensure!(
                    request.version == VERSION && request.scope == scope && request.token == token,
                    Message::ControlProtocol
                );
                let mut response = Response {
                    version: VERSION,
                    accepted: true,
                    status: None,
                };
                match request.action {
                    Command::Wake => {
                        let _ = sender.try_send(Signal::Wake);
                    }
                    Command::Stop => {
                        response.accepted = sender.send(Signal::Stop).await.is_ok();
                    }
                    Command::ManagedRunning => {
                        response.accepted = managed;
                    }
                    Command::ManagedStop if managed => {
                        response.accepted = sender.send(Signal::Stop).await.is_ok();
                    }
                    Command::Status => {
                        response.status = snapshot.read().ok().map(|value| value.clone());
                    }
                    Command::SessionReady { session } if scope == "user" => {
                        response.accepted = !session.is_empty()
                            && session.len() <= 256
                            && !session.chars().any(char::is_control);
                        if response.accepted {
                            response.accepted =
                                sender.send(Signal::SessionReady(session)).await.is_ok();
                        }
                    }
                    Command::SessionEnd { session }
                        if scope == "user"
                            && !session.is_empty()
                            && session.len() <= 256
                            && !session.chars().any(char::is_control) =>
                    {
                        response.accepted = sender.send(Signal::SessionEnd(session)).await.is_ok();
                    }
                    _ => response.accepted = false,
                }
                // Acknowledgement proves the command passed validation and was accepted.
                protocol::write(&mut socket, &response).await
            })
            .await;
        }
    });
    Ok((
        Server {
            task,
            path,
            status,
            #[cfg(unix)]
            socket: paths.runtime.join("daemon.sock"),
        },
        receiver,
    ))
}

async fn request(paths: &Paths, action: Command) -> Result<Response> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
        ensure!(
            endpoint.version == VERSION && endpoint.scope == paths.scope.label(),
            Message::ControlProtocol
        );
        let mut socket = transport::connect(&endpoint.address).await?;
        protocol::write(
            &mut socket,
            &Request {
                version: VERSION,
                scope: endpoint.scope,
                token: endpoint.token,
                action,
            },
        )
        .await?;
        let response: Response = protocol::read(&mut socket).await?;
        ensure!(
            response.version == VERSION && response.accepted,
            Message::ControlProtocol
        );
        Ok(response)
    })
    .await
    .map_err(|_| crate::infra::error::error!(Message::ControlProtocol))?
}
pub async fn wake(paths: &Paths) {
    let _ = request(paths, Command::Wake).await;
}
pub async fn stop(paths: &Paths) {
    let _ = request(paths, Command::Stop).await;
}
pub async fn status(paths: &Paths) -> Result<RuntimeStatus> {
    request(paths, Command::Status)
        .await?
        .status
        .ok_or_else(|| crate::infra::error::error!(Message::ControlProtocol))
}
#[cfg(target_os = "linux")]
pub async fn session_ready(paths: &Paths, session: String) -> Result<()> {
    request(paths, Command::SessionReady { session })
        .await
        .map(|_| ())
}
#[cfg(target_os = "linux")]
pub async fn session_end(paths: &Paths, session: String) -> Result<()> {
    request(paths, Command::SessionEnd { session })
        .await
        .map(|_| ())
}

#[cfg(target_os = "macos")]
pub async fn managed_running(paths: &Paths) -> bool {
    request(paths, Command::ManagedRunning).await.is_ok()
}
#[cfg(target_os = "macos")]
pub async fn managed_pid(paths: &Paths) -> Option<u32> {
    let endpoint: Endpoint =
        serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json")).ok()?).ok()?;
    if endpoint.pid > 0 && managed_running(paths).await {
        Some(endpoint.pid)
    } else {
        None
    }
}
#[cfg(target_os = "macos")]
pub async fn stop_managed(paths: &Paths) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let _ = request(paths, Command::ManagedStop).await;
            if super::lock::managed_lock(paths).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| crate::infra::error::error!(Message::DaemonHasNotExitedStopTheRunningDaemon))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn status_and_session_notifications_are_authenticated_versioned_and_scope_bound()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::test(directory.path().join("user"));
        let (server, mut commands) = serve(&paths).await?;
        if let Ok(mut status) = server.status.write() {
            status.credentials = CredentialState::WaitingForUnlock;
        }
        assert_eq!(
            status(&paths).await?.credentials,
            CredentialState::WaitingForUnlock
        );
        assert!(commands.try_recv().is_err());
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
        for (version, scope) in [(VERSION + 1, "user"), (VERSION, "system")] {
            let mut socket = transport::connect(&endpoint.address).await?;
            protocol::write(
                &mut socket,
                &Request {
                    version,
                    scope: scope.into(),
                    token: endpoint.token.clone(),
                    action: Command::Stop,
                },
            )
            .await?;
            assert!(protocol::read::<Response>(&mut socket).await.is_err());
            assert!(commands.try_recv().is_err());
        }
        request(
            &paths,
            Command::SessionReady {
                session: "session-1".into(),
            },
        )
        .await?;
        assert_eq!(
            commands.recv().await,
            Some(Signal::SessionReady("session-1".into()))
        );
        request(
            &paths,
            Command::SessionEnd {
                session: "session-1".into(),
            },
        )
        .await?;
        assert_eq!(
            commands.recv().await,
            Some(Signal::SessionEnd("session-1".into()))
        );
        let mut other = Paths::test(directory.path().join("system"));
        other.scope = crate::infra::scope::Scope::System;
        let (_system, mut system_commands) = serve(&other).await?;
        assert!(
            request(
                &other,
                Command::SessionReady {
                    session: "session-1".into()
                }
            )
            .await
            .is_err()
        );
        wake(&paths).await;
        assert_eq!(commands.recv().await, Some(Signal::Wake));
        assert!(system_commands.try_recv().is_err());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_stop_does_not_stop_a_foreground_replacement() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let (server, mut receiver) = serve_with_mode(&paths, true).await?;
        assert!(managed_running(&paths).await);
        assert!(request(&paths, Command::ManagedStop).await.is_ok());
        assert_eq!(receiver.recv().await, Some(Signal::Stop));
        drop(server);
        let (_manual, mut receiver) = serve_with_mode(&paths, false).await?;
        assert!(!managed_running(&paths).await);
        stop_managed(&paths).await?;
        assert!(receiver.try_recv().is_err());
        wake(&paths).await;
        assert_eq!(receiver.recv().await, Some(Signal::Wake));
        Ok(())
    }

    #[tokio::test]
    async fn local_ipc_requires_token_and_wakes_daemon() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let (_server, mut receiver) = serve(&paths).await?;
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
        let mut stranger = transport::connect(&endpoint.address).await?;
        protocol::write(
            &mut stranger,
            &Request {
                version: VERSION,
                scope: "user".into(),
                token: "x".repeat(64),
                action: Command::Wake,
            },
        )
        .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                .await
                .is_err()
        );
        wake(&paths).await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await?,
            Some(Signal::Wake)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(paths.runtime.join("daemon.sock"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }
}
