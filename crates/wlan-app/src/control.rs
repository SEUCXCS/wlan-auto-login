use crate::config::{Paths, private_dir, write_private};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub paused: bool,
    pub session: Option<String>,
    pub revision: u64,
}

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("control.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("后台控制状态损坏"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("control.json"),
            &serde_json::to_vec(self)?,
        )
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.revision = self.revision.wrapping_add(1);
    }
}

fn open_lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

pub fn singleton(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.config.join("daemon.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context("daemon 已在运行")?;
    Ok(file)
}

pub fn service_lock(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.config.join("service.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context("另一项服务安装或卸载操作正在运行")?;
    Ok(file)
}

pub async fn operation_lock(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.config.join("operation.lock"))?;
    for _ in 0..250 {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("另一项登录或注销操作尚未结束，请稍后重试")
}

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
    getrandom::fill(&mut random).map_err(|e| anyhow::anyhow!("无法生成控制令牌: {e}"))?;
    let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
    private_dir(&paths.config)?;
    let (mut listener, address) = crate::transport::Listener::bind(paths, &token)?;
    let endpoint = Endpoint {
        address,
        token: token.clone(),
    };
    let path = paths.config.join("daemon-endpoint.json");
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
            socket: paths.config.join("daemon.sock"),
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
            serde_json::from_slice(&fs::read(paths.config.join("daemon-endpoint.json"))?)?;
        let mut socket = crate::transport::connect(&endpoint.address).await?;
        socket.write_all(endpoint.token.as_bytes()).await?;
        socket.write_all(&[command]).await?;
        Ok::<(), anyhow::Error>(())
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_ipc_requires_token_and_wakes_daemon() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config: dir.path().into(),
        };
        let (_server, mut receiver) = serve(&paths).await?;
        let endpoint: Endpoint =
            serde_json::from_slice(&fs::read(paths.config.join("daemon-endpoint.json"))?)?;
        let mut stranger = crate::transport::connect(&endpoint.address).await?;
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
                fs::metadata(paths.config.join("daemon.sock"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }
    #[tokio::test]
    async fn instance_lock_is_exclusive_and_released_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config: dir.path().into(),
        };
        let lock = singleton(&paths)?;
        assert!(singleton(&paths).is_err());
        drop(lock);
        assert!(singleton(&paths).is_ok());
        Ok(())
    }
    #[test]
    fn pause_survives_restart_and_explicit_login_resumes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config: dir.path().into(),
        };
        State {
            paused: true,
            session: Some("connection-1".into()),
            revision: 0,
        }
        .save(&paths)?;
        let mut state = State::load(&paths)?;
        assert!(state.paused);
        state.resume();
        state.save(&paths)?;
        assert!(!State::load(&paths)?.paused);
        assert_eq!(State::load(&paths)?.revision, 1);
        Ok(())
    }
}
