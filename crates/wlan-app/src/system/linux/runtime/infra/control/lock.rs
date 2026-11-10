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

    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
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
                    || (false
                        && e.raw_os_error() == fs2::lock_contended_error().raw_os_error()) =>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn contended_operation_waits_until_the_lock_is_released() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let held = operation_lock(&paths).await?;
        let waiting = operation_lock(&paths);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        drop(held);
        let _acquired = tokio::time::timeout(Duration::from_secs(1), waiting).await??;
        Ok(())
    }

    #[tokio::test]
    async fn instance_lock_is_exclusive_and_released_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let lock = singleton(&paths)?;
        assert!(singleton(&paths).is_err());
        drop(lock);
        assert!(singleton(&paths).is_ok());
        Ok(())
    }
}
