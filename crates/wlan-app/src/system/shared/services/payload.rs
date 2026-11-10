//! Byte-preserving backups shared by platform-owned installation transactions.
use crate::infra::error::{Context, Message, Result};
use std::{fs, path::Path};
fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub(crate) struct Backup(Option<tempfile::NamedTempFile>);
#[allow(dead_code)] // The signed macOS payload uses its own backup transaction.
impl Backup {
    pub(crate) fn capture(path: &Path) -> Result<Self> {
        match fs::metadata(path) {
            Ok(_) => {
                let backup = tempfile::NamedTempFile::new_in(
                    path.parent()
                        .context(Message::BackupPathHasNoParentDirectory)?,
                )?;
                fs::copy(path, backup.path())?;
                Ok(Self(Some(backup)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self(None)),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn existed(&self) -> bool {
        self.0.is_some()
    }
    pub(crate) fn retain(mut self) -> Result<Option<std::path::PathBuf>> {
        self.0
            .take()
            .map(|backup| {
                backup
                    .keep()
                    .map(|(_, path)| path)
                    .map_err(|e| e.error.into())
            })
            .transpose()
    }
    pub(crate) fn restore(&self, path: &Path) -> Result<()> {
        match &self.0 {
            Some(backup) => {
                fs::copy(backup.path(), path)?;
                Ok(())
            }
            None => remove_if_present(path),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backups_restore_existing_files_and_remove_new_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("file");
        let absent = Backup::capture(&file)?;
        fs::write(&file, "original")?;
        let backup = Backup::capture(&file)?;
        fs::write(&file, "replacement")?;
        backup.restore(&file)?;
        assert_eq!(fs::read_to_string(&file)?, "original");
        absent.restore(&file)?;
        assert!(!file.exists());
        Ok(())
    }
}
