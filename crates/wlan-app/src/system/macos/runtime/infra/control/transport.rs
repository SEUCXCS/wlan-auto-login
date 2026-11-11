//! Local IPC restricted to the selected user or system scope. No TCP port is opened.

use crate::infra::error::Message;
use crate::infra::error::Result;
use crate::infra::paths::Paths;

pub type Stream = tokio::net::UnixStream;

pub struct Listener {
    inner: tokio::net::UnixListener,
}

impl Listener {
    pub fn bind(paths: &Paths, _nonce: &str) -> Result<(Self, String)> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let path = paths.runtime.join("daemon.sock");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&path)?,
            Ok(_) => crate::infra::error::bail!(Message::ControlSocketPathIsOccupiedByAnotherFile),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let inner = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok((Self { inner }, path.to_string_lossy().into_owned()))
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        Ok(self.inner.accept().await?.0)
    }
}

pub async fn connect(address: &str) -> Result<Stream> {
    Ok(Stream::connect(address).await?)
}
