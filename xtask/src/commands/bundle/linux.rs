//! Helpers shared by the Debian and RPM package backends.
use anyhow::{Context, Result, ensure};
use std::{
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

/// Package changelog entries carry the date of the version's CHANGELOG.md entry.
pub(super) fn release_date<'a>(changelog: &'a str, version: &str) -> Result<&'a str> {
    let heading = format!("## [{version}] - ");
    changelog
        .lines()
        .find_map(|line| line.strip_prefix(heading.as_str()))
        .map(str::trim)
        .with_context(|| format!("CHANGELOG.md has no \"## [{version}] - YYYY-MM-DD\" entry"))
}

/// Format a CHANGELOG.md date in UTC with a date(1) `+FORMAT` string.
pub(super) fn format_date(date: &str, format: &str) -> Result<String> {
    let output = Command::new("date")
        .env("LC_ALL", "C")
        .args(["--utc", "--date", date, format])
        .output()
        .context("Failed to run date")?;
    ensure!(
        output.status.success(),
        "CHANGELOG.md has an invalid release date: {date}"
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
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
    fn release_date_matches_the_exact_version() -> Result<()> {
        let changelog =
            "## [Unreleased]\n\n## [0.2.0-alpha.1] - 2026-11-20\n\n## [0.2.0] - 2026-11-25\n";
        assert_eq!(release_date(changelog, "0.2.0")?, "2026-11-25");
        assert_eq!(release_date(changelog, "0.2.0-alpha.1")?, "2026-11-20");
        assert!(release_date(changelog, "0.3.0").is_err());
        Ok(())
    }
}
