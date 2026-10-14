//! Locate workspace resources independently of the caller's working directory.
use anyhow::{Context, Result};
use std::path::Path;

pub fn root() -> Result<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Workspace root is missing")
}
