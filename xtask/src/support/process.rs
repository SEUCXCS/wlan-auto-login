//! Execute external tools with contextual failures.
use anyhow::{Context, Result, ensure};
use std::process::Command;

pub fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("Failed to run {}", command.get_program().to_string_lossy()))?;
    ensure!(
        status.success(),
        "{} failed: {status}",
        command.get_program().to_string_lossy()
    );
    Ok(())
}
