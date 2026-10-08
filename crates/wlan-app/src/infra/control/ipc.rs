use super::transport;
use crate::infra::error::Message;
use crate::infra::error::Result;
use crate::infra::{
    filesystem::{private_dir, write_private},
    paths::Paths,
};
use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

#[derive(Serialize, Deserialize)]
struct Endpoint {
    address: String,
    token: String,
    #[serde(default)]
    pid: u32,
}

pub struct Server {
    task: tokio::task::JoinHandle<()>,
    path: std::path::PathBuf,
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
}

#[cfg(test)]
pub async fn serve(paths: &Paths) -> Result<(Server, mpsc::Receiver<Signal>)> {
    serve_with_mode(paths, false).await
}

pub async fn serve_with_mode(
    paths: &Paths,
    managed: bool,
) -> Result<(Server, mpsc::Receiver<Signal>)> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|e| {
        crate::infra::error::error!(Message::FailedToGenerateControlTokenE { e: (e).to_string() })
    })?;
    let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
    private_dir(&paths.runtime)?;
    let (mut listener, address) = transport::Listener::bind(paths, &token)?;
    let endpoint = Endpoint {
        address,
        token: token.clone(),
        pid: std::process::id(),
    };
    let path = paths.runtime.join("daemon-endpoint.json");
    write_private(&path, &serde_json::to_vec(&endpoint)?)?;
    let (sender, receiver) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        while let Ok(mut socket) = listener.accept().await {
            let mut bytes = [0u8; 64];
            if matches!(
                tokio::time::timeout(Duration::from_secs(1), socket.read_exact(&mut bytes)).await,
                Ok(Ok(_))
            ) && bytes == token.as_bytes()
            {
                let mut command = [0];
                if matches!(
                    tokio::time::timeout(Duration::from_secs(1), socket.read_exact(&mut command))
                        .await,
                    Ok(Ok(_))
                ) {
                    match command[0] {
                        b'w' => {
                            let _ = sender.try_send(Signal::Wake);
                        }
                        b'p' if managed => {}
                        b'm' if managed => {
                            let _ = sender.send(Signal::Stop).await;
                        }
                        b's' => {
                            let _ = sender.send(Signal::Stop).await;
                        }
                        _ => continue,
                    }
                }
                let _ = socket.write_all(b"ok").await;
            }
        }
    });
    Ok((
        Server {
            task,
            path,
            #[cfg(unix)]
            socket: paths.runtime.join("daemon.sock"),
        },
        receiver,
    ))
}

pub async fn wake(paths: &Paths) {
    send(paths, b'w').await;
}
pub async fn stop(paths: &Paths) {
    send(paths, b's').await;
}
async fn send(paths: &Paths, command: u8) {
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
        let mut socket = transport::connect(&endpoint.address).await?;
        socket.write_all(endpoint.token.as_bytes()).await?;
        socket.write_all(&[command]).await?;
        Ok::<(), crate::infra::error::Error>(())
    })
    .await;
}

/// The server checks its own launch mode, so stale metadata can never stop a
/// replacement foreground daemon. No PID-based signalling is used.
#[cfg(target_os = "macos")]
async fn managed_command(paths: &Paths, command: u8) -> bool {
    tokio::time::timeout(Duration::from_secs(1), async {
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
        let mut socket = transport::connect(&endpoint.address).await?;
        socket.write_all(endpoint.token.as_bytes()).await?;
        socket.write_all(&[command]).await?;
        let mut ack = [0; 2];
        socket.read_exact(&mut ack).await?;
        Ok::<_, crate::infra::error::Error>(ack == *b"ok")
    })
    .await
    .is_ok_and(|result| result.unwrap_or(false))
}
#[cfg(target_os = "macos")]
pub async fn managed_running(paths: &Paths) -> bool {
    managed_command(paths, b'p').await
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
            managed_command(paths, b'm').await;
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
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_stop_does_not_stop_a_foreground_replacement() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let (server, mut receiver) = serve_with_mode(&paths, true).await?;
        assert!(managed_running(&paths).await);
        assert!(managed_command(&paths, b'm').await);
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
        stranger.write_all(&[b'x'; 64]).await?;
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
