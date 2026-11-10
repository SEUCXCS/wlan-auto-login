use crate::infra::error::Message;
use crate::infra::error::{Context, Result};

use std::fs;
use std::path::Path;

pub fn private_dir(path: &Path) -> Result<()> {
    {
        fs::create_dir_all(path)?;
        let metadata = fs::symlink_metadata(path)?;
        crate::infra::error::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            Message::ConfigurationDirectoryMustNotBeASymbolic {
                value0: (path.display()).to_string()
            }
        );

        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            crate::infra::error::ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                Message::ConfigurationDirectoryIsNotOwnedByThe {
                    value0: (path.display()).to_string()
                }
            );
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context(Message::FileHasNoParentDirectory)?;
    private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
