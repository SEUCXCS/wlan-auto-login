//! Scope-safe action logs live in the private directory prepared by the immediate MSI action.
use crate::{validate_id, win};
use anyhow::{Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Diagnostics {
    directory: PathBuf,
}
impl Diagnostics {
    pub fn open(directory: &str, transaction: &str) -> Result<Self> {
        validate_id(transaction)?;
        let directory = PathBuf::from(directory);
        ensure!(
            directory
                .file_name()
                .is_some_and(|name| name == format!("wlanlogin-form-{transaction}").as_str()),
            "Unexpected diagnostics directory"
        );
        win::plain_path(&directory)?;
        ensure!(
            directory.is_dir(),
            "MSI diagnostics directory is unavailable"
        );
        Ok(Self { directory })
    }
    pub fn stage(&self, text: &str) -> Result<()> {
        let path = self.directory.join("action.log");
        win::plain_path(&path)?;
        let mut output = OpenOptions::new().append(true).create(true).open(path)?;
        writeln!(output, "{text}")?;
        Ok(())
    }
    pub fn cli_log(&self) -> PathBuf {
        self.directory.join("cli.log")
    }
    pub fn cleanup(&self) -> Result<()> {
        let entries = fs::read_dir(&self.directory)?.collect::<std::io::Result<Vec<_>>>()?;
        for entry in &entries {
            ensure!(
                ["action.log", "cli.log", "credentials.bin"]
                    .iter()
                    .any(|name| entry.file_name() == *name),
                "Unknown diagnostic file; directory retained"
            );
            win::plain_path(&entry.path())?;
        }
        for entry in entries {
            fs::remove_file(entry.path())?;
        }
        fs::remove_dir(&self.directory)?;
        Ok(())
    }
}

pub fn report_error(arguments: &[String], error: &anyhow::Error) {
    if let [action, scope, transaction, _, _, directory] = arguments
        && let Ok(log) = Diagnostics::open(directory, transaction)
    {
        let _ = log.stage(&format!("action={action} scope={scope} failed: {error:#}"));
    }
}

pub fn log_file(directory: &Path) -> PathBuf {
    directory.join("setup.log")
}
