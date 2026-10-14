//! Replace verified bundles while preserving the previous output on failure.
use anyhow::{Context, Result, bail, ensure};
use std::{fs, path::Path};

use crate::support::identity;

pub fn publish(app: &Path, destination: &Path, staging: &Path) -> Result<()> {
    let backup = staging.join("previous.app");
    let existed = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Refusing to replace a redirected or non-directory bundle"
            );
            let previous = fs::read_to_string(destination.join("Contents/Info.plist"))?;
            ensure!(
                previous.contains(&format!("<string>{}</string>", identity::APP_ID)),
                "Refusing to replace an unrelated bundle"
            );
            fs::rename(destination, &backup)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = fs::rename(app, destination) {
        if existed && let Err(rollback) = fs::rename(&backup, destination) {
            // The caller must retain the temporary directory if rollback ever fails.
            let recovery = destination.with_extension("app.recovery");
            fs::rename(&backup, &recovery)
                .context("Failed to preserve previous bundle during rollback")?;
            bail!(
                "Bundle publication failed: {error}; rollback failed: {rollback}; previous bundle: {}",
                recovery.display()
            );
        }
        return Err(error).context("Failed to publish bundle; previous bundle restored");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_publication_restores_the_previous_bundle() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("wlanlogin.app");
        fs::create_dir_all(destination.join("Contents"))?;
        fs::write(
            destination.join("Contents/Info.plist"),
            format!("<string>{}</string>", identity::APP_ID),
        )?;
        fs::write(destination.join("previous-version"), "keep")?;
        assert!(publish(&root.path().join("missing.app"), &destination, root.path()).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("previous-version"))?,
            "keep"
        );
        Ok(())
    }
}
