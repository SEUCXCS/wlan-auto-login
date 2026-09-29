//! Whole-bundle staging. Only the exact bundle layout emitted by xtask is owned.
use crate::infra::command::Invocation;
use crate::infra::{
    cleanup,
    error::{Context, Result, ensure},
    identity::APP_ID,
};
use crate::system::macos::bundle;
use std::{
    fs,
    path::{Path, PathBuf},
};

const FILES: &[&str] = &[
    "Contents/Info.plist",
    "Contents/PkgInfo",
    "Contents/MacOS/wlanlogin",
    "Contents/_CodeSignature/CodeResources",
];
const DIRECTORIES: &[&str] = &["Contents", "Contents/MacOS", "Contents/_CodeSignature"];

pub fn inspect(app: &Path) -> Result<()> {
    cleanup::validate_directory(app)?;
    fn visit(root: &Path, dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .context("Bundle member escaped its root")?
                .to_str()
                .context("Non-UTF-8 bundle member")?
                .to_owned();
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Refusing a symlink in bundle: {}",
                path.display()
            );
            if kind.is_dir() {
                ensure!(
                    DIRECTORIES.contains(&relative.as_str()),
                    "Unmanaged bundle directory retained: {}",
                    path.display()
                );
                visit(root, &path)?;
            } else {
                ensure!(
                    kind.is_file() && FILES.contains(&relative.as_str()),
                    "Unmanaged bundle file retained: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }
    visit(app, app)
}

pub async fn validate(app: &Path) -> Result<()> {
    inspect(app)?;
    ensure!(
        bundle::executable(app).is_file(),
        "Bundle executable is missing"
    );
    let plist = app.join("Contents/Info.plist");
    for (key, expected) in [
        ("CFBundleIdentifier", APP_ID),
        ("CFBundleExecutable", "wlanlogin"),
    ] {
        let cmd = Invocation::new(
            "/usr/bin/plutil",
            [
                std::ffi::OsStr::new("-extract"),
                std::ffi::OsStr::new(key),
                std::ffi::OsStr::new("raw"),
                std::ffi::OsStr::new("-o"),
                std::ffi::OsStr::new("-"),
                plist.as_os_str(),
            ],
        );
        let output = cmd.output().await?;
        output.check(&cmd)?;
        ensure!(
            output.stdout.trim() == expected,
            "Unexpected bundle identity: {key}"
        );
    }
    Invocation::new(
        "/usr/bin/codesign",
        [
            std::ffi::OsStr::new("--verify"),
            std::ffi::OsStr::new("--deep"),
            std::ffi::OsStr::new("--strict"),
            app.as_os_str(),
        ],
    )
    .run()
    .await
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let to = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    use std::os::unix::fs::PermissionsExt;
    // System Wi-Fi helpers run as the console user and must traverse the bundle.
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

pub struct Payload {
    storage: Option<tempfile::TempDir>,
    destination: PathBuf,
    directory: PathBuf,
    published: bool,
    had_previous: bool,
}
impl Payload {
    pub fn stage(source: &Path, destination: &Path) -> Result<Self> {
        inspect(source)?;
        let parent = destination
            .parent()
            .context("Bundle destination has no parent")?;
        cleanup::validate_directory(parent)?;
        fs::create_dir_all(parent)?;
        if destination.try_exists()? {
            inspect(destination)?;
        }
        let storage = tempfile::Builder::new()
            .prefix(".bundle-transaction-")
            .tempdir_in(parent)?;
        copy_tree(source, &storage.path().join("new.app"))?;
        Ok(Self {
            directory: storage.path().to_owned(),
            storage: Some(storage),
            destination: destination.to_owned(),
            published: false,
            had_previous: false,
        })
    }
    fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn recovery_path(&self, name: &str) -> PathBuf {
        self.directory().join(name)
    }
    pub fn staged(&self) -> PathBuf {
        self.directory().join("new.app")
    }
    pub fn existed(&self) -> bool {
        self.had_previous
    }
    pub fn publish(&mut self) -> Result<()> {
        if self.destination.try_exists()? {
            fs::rename(&self.destination, self.directory().join("old.app"))?;
            self.had_previous = true;
        }
        // Mark before the rename so a failure between the two renames is recoverable.
        self.published = true;
        fs::rename(self.staged(), &self.destination)?;
        Ok(())
    }
    pub fn restore(&mut self) -> Result<()> {
        if !self.published {
            return Ok(());
        }
        if self.destination.try_exists()? {
            inspect(&self.destination)?;
            fs::rename(&self.destination, self.directory().join("failed.app"))?;
        }
        if self.had_previous {
            fs::rename(self.directory().join("old.app"), &self.destination)?;
        }
        self.published = false;
        Ok(())
    }
    pub fn retain(&mut self) -> PathBuf {
        self.published = false;
        if let Some(storage) = self.storage.take() {
            let _ = storage.keep();
        }
        self.directory.clone()
    }
    pub fn commit(mut self) {
        self.published = false;
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        // Cancellation or an unhandled early return must never discard rollback data.
        if self.published
            && let Some(storage) = self.storage.take()
        {
            let backup = storage.keep();
            tracing::error!(backup = %backup.display(), "Interrupted bundle transaction; recovery files retained");
        }
    }
}

pub fn remove(app: &Path) -> Result<()> {
    if !app.try_exists()? {
        return Ok(());
    }
    inspect(app)?;
    // No recursive removal of unknown files or symlinks. A concurrent addition
    // makes remove_dir fail and stays on disk.
    for name in FILES {
        cleanup::remove_file(&app.join(name))?;
    }
    for name in DIRECTORIES.iter().rev() {
        match fs::remove_dir(app.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    fs::remove_dir(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(app: &Path, value: &str) -> Result<()> {
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(bundle::executable(app), value)?;
        fs::write(app.join("Contents/Info.plist"), value)?;
        Ok(())
    }
    #[test]
    fn rollback_restores_the_entire_bundle_and_first_install_disappears() -> Result<()> {
        for previous in [false, true] {
            let root = tempfile::tempdir()?;
            let source = root.path().join("source.app");
            let destination = root.path().join("installed.app");
            fixture(&source, "new")?;
            if previous {
                fixture(&destination, "old")?;
            }
            let mut tx = Payload::stage(&source, &destination)?;
            tx.publish()?;
            assert_eq!(fs::read_to_string(bundle::executable(&destination))?, "new");
            tx.restore()?;
            if previous {
                assert_eq!(fs::read_to_string(bundle::executable(&destination))?, "old");
                assert_eq!(
                    fs::read_to_string(destination.join("Contents/Info.plist"))?,
                    "old"
                );
            } else {
                assert!(!destination.exists());
            }
        }
        Ok(())
    }
    #[test]
    fn unknown_files_and_redirected_members_are_retained() -> Result<()> {
        let root = tempfile::tempdir()?;
        let app = root.path().join("test.app");
        fixture(&app, "old")?;
        fs::write(app.join("personal-note"), "preserve")?;
        assert!(remove(&app).is_err());
        assert!(bundle::executable(&app).exists());
        fs::remove_file(app.join("personal-note"))?;
        fs::remove_file(bundle::executable(&app))?;
        let outside = root.path().join("outside");
        fs::write(&outside, "preserve")?;
        std::os::unix::fs::symlink(&outside, bundle::executable(&app))?;
        assert!(remove(&app).is_err());
        assert_eq!(fs::read_to_string(outside)?, "preserve");
        Ok(())
    }
    #[test]
    fn interrupted_transaction_keeps_previous_bundle() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.app");
        let destination = root.path().join("installed.app");
        fixture(&source, "new")?;
        fixture(&destination, "old")?;
        let mut tx = Payload::stage(&source, &destination)?;
        tx.publish()?;
        let backup = tx.directory().to_owned();
        drop(tx);
        assert_eq!(
            fs::read_to_string(bundle::executable(&backup.join("old.app")))?,
            "old"
        );
        Ok(())
    }
}
