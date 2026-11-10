use crate::infra::error::Message;
use crate::infra::error::{Context, Result, bail};
use crate::infra::{filesystem::private_dir, paths::Paths};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

fn open_lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);

    Ok(options.open(path)?)
}

pub fn singleton(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.runtime.join("daemon.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context(Message::DaemonIsAlreadyRunning)?;
    Ok(file)
}

pub fn service_lock(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.config.join("service.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context(Message::AnotherServiceManagementOperationIsInProgress)?;
    Ok(file)
}

pub async fn operation_lock(paths: &Paths) -> Result<File> {
    let file = open_lock(&paths.runtime.join("operation.lock"))?;
    for _ in 0..250 {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || (true && e.raw_os_error() == fs2::lock_contended_error().raw_os_error()) =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!(Message::AnotherAuthenticationOperationIsInProgressRetryLater)
}

pub async fn wait_stopped(paths: &Paths) -> Result<()> {
    for _ in 0..250 {
        if singleton(paths).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!(Message::DaemonHasNotExitedStopTheRunningDaemon)
}
