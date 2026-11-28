//! Helpers shared by the Debian and RPM package backends.
use anyhow::{Context, Result, ensure};
use std::{
    env,
    ffi::OsStr,
    fs,
    path::Path,
    process::{Command, Stdio},
};

use crate::support::platform;

/// Fail before compiling when the host lacks a packaging tool.
pub(super) fn require_tools(tools: &[&str], hint: &str) -> Result<()> {
    for tool in tools {
        let available = Command::new(tool)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        ensure!(available, "{tool} is required; {hint}");
    }
    Ok(())
}

/// Format the package changelog date in UTC with a date(1) `+FORMAT` string.
pub(super) fn package_date(root: &Path, format: &str) -> Result<String> {
    let epoch = env::var_os("SOURCE_DATE_EPOCH");
    let timestamp = package_timestamp(root, epoch.as_deref())?;
    let output = Command::new("date")
        .env("LC_ALL", "C")
        .args(["--utc", "--date", &format!("@{timestamp}"), format])
        .output()
        .context("Failed to run date")?;
    ensure!(
        output.status.success(),
        "Failed to format package changelog date for Unix timestamp {timestamp}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

fn package_timestamp(root: &Path, source_date_epoch: Option<&OsStr>) -> Result<u64> {
    if let Some(epoch) = source_date_epoch {
        return epoch
            .to_str()
            .and_then(|value| value.parse().ok())
            .context("SOURCE_DATE_EPOCH must be a non-negative integer Unix timestamp");
    }

    // An archive extracted inside another checkout must not inherit its HEAD date.
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "--git-dir=.git",
            "log",
            "-1",
            "--no-show-signature",
            "--format=%ct",
            "HEAD",
        ])
        .output()
        .context("Failed to run Git for the package changelog date. Set SOURCE_DATE_EPOCH")?;
    ensure!(
        output.status.success(),
        "Cannot read the source commit date. Set SOURCE_DATE_EPOCH: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout)?
        .trim()
        .parse()
        .context("Git returned an invalid commit timestamp. Set SOURCE_DATE_EPOCH")
}

pub(super) fn stage(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    create_parent(destination)?;
    fs::copy(source, destination)
        .with_context(|| format!("Failed to stage {}", source.display()))?;
    platform::set_mode(destination, mode)
}

pub(super) fn write_file(destination: &Path, contents: &str) -> Result<()> {
    create_parent(destination)?;
    fs::write(destination, contents)?;
    platform::set_mode(destination, 0o644)
}

fn create_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("Package path has no parent")?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_date_epoch_allows_packaging_without_git() -> Result<()> {
        let source = tempfile::tempdir()?;
        assert_eq!(
            package_timestamp(source.path(), Some(OsStr::new("1795651200")))?,
            1_795_651_200
        );
        for invalid in ["", "-1", "2026-11-26"] {
            assert!(package_timestamp(source.path(), Some(OsStr::new(invalid))).is_err());
        }
        assert!(matches!(
            package_timestamp(source.path(), None),
            Err(error) if error.to_string().contains("Set SOURCE_DATE_EPOCH")
        ));
        Ok(())
    }
}
