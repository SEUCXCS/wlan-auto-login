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

pub async fn serve(paths: &Paths) -> Result<(Server, mpsc::Receiver<Signal>)> {
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

#[cfg(test)]
mod tests {
    use super::*;
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
