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

    // An archive extracted inside another checkout must not inherit its HEAD date,
    // while git worktrees and submodules use a `.git` file rather than a directory.
    ensure!(
        root.join(".git").exists(),
        "Cannot read the source commit date: no .git entry in {}. Set SOURCE_DATE_EPOCH",
        root.display()
    );
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args(["log", "-1", "--no-show-signature", "--format=%ct", "HEAD"]);
    if let Some(parent) = root.parent() {
        command.env("GIT_CEILING_DIRECTORIES", parent);
    }
    let output = command
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

    #[test]
    fn git_timestamp_supports_worktrees_and_ignores_parent_repositories() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let repo = sandbox.path().join("repo");
        let worktree = sandbox.path().join("worktree");
        fs::create_dir(&repo)?;

        let run_git = |dir: &Path, args: &[&str]| -> Result<()> {
            let output = Command::new("git")
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_AUTHOR_DATE", "1795651200 +0000")
                .env("GIT_COMMITTER_NAME", "test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_DATE", "1795651200 +0000")
                .args(args)
                .output()?;
            ensure!(
                output.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        };

        run_git(&repo, &["init", "-q"])?;
        fs::write(repo.join("README.md"), "test\n")?;
        run_git(&repo, &["add", "README.md"])?;
        run_git(&repo, &["commit", "-q", "--no-gpg-sign", "-m", "init"])?;
        let worktree_path = worktree.to_str().context("worktree path is not UTF-8")?;
        run_git(&repo, &["worktree", "add", "-q", "--detach", worktree_path])?;

        assert!(worktree.join(".git").is_file());
        assert_eq!(package_timestamp(&repo, None)?, 1_795_651_200);
        assert_eq!(package_timestamp(&worktree, None)?, 1_795_651_200);

        let nested = repo.join("extracted-archive");
        fs::create_dir(&nested)?;
        assert!(matches!(
            package_timestamp(&nested, None),
            Err(error) if error.to_string().contains("Set SOURCE_DATE_EPOCH")
        ));
        Ok(())
    }
}
