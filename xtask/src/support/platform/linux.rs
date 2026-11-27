use anyhow::{Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn require_linux() -> Result<()> {
    Ok(())
}

/// Package payloads must not inherit the caller's umask.
pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set the mode of {}", path.display()))
}
