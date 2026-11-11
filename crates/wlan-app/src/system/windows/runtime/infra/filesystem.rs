use crate::infra::error::Message;
use crate::infra::error::{Context, Result};

use std::path::Path;

#[path = "filesystem_windows.rs"]
pub(crate) mod windows;

pub fn private_dir(path: &Path) -> Result<()> {
    {
        windows::prepare_directory(path, false)?;
        Ok(())
    }
}

pub fn restrict_windows(path: &Path, public: bool) -> Result<()> {
    windows::prepare_directory(path, public)?;
    Ok(())
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

#[cfg(test)]
pub(crate) mod tests {
    /// Junctions exercise reparse-point checks without requiring symlink privileges.
    pub(crate) fn redirect_directory(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> crate::infra::error::Result<()> {
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;
        crate::infra::error::ensure!(
            output.status.success(),
            "cannot create test directory junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
}
