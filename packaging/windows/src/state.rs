//! Scope-specific snapshots, without changing the installed CLI's format or paths.
use crate::{
    Scope,
    credentials::{self, Credential},
    identity,
    services::{self, Registration},
    win,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Paths {
    pub config: PathBuf,
    pub binary: PathBuf,
    pub definition: PathBuf,
    pub secrets: PathBuf,
    pub install: PathBuf,
}
impl Paths {
    pub fn new(scope: Scope) -> Result<Self> {
        Self::resolve(scope, || {
            let user = directories::ProjectDirs::from_path(
                PathBuf::from(identity::ORGANIZATION).join(identity::APP_NAME),
            )
            .context("User folders unavailable")?;
            let base = directories::BaseDirs::new().context("Windows profile unavailable")?;
            Ok((user, base))
        })
    }
    fn resolve(
        scope: Scope,
        user_folders: impl FnOnce() -> Result<(directories::ProjectDirs, directories::BaseDirs)>,
    ) -> Result<Self> {
        // LocalSystem has no interactive profile. Machine paths use only global Known Folders.
        if scope == Scope::Machine {
            let data = win::program_data()?.join("SEUCXCS/wlanlogin");
            let install = win::program_files()?.join("SEUCXCS/wlanlogin");
            return Ok(Self {
                config: data.join("state"),
                secrets: data.join("secrets"),
                definition: data.join("service.json"),
                binary: install.join("wlanlogin-daemon.exe"),
                install,
            });
        }
        let (user, base) = user_folders()?;
        let config = user.config_dir().to_path_buf();
        Ok(Self {
            definition: config.join("daemon-task.xml"),
            secrets: config.clone(),
            config,
            binary: user.data_local_dir().join("bin/wlanlogin-daemon.exe"),
            install: base.data_local_dir().join("Programs/SEUCXCS/wlanlogin"),
        })
    }
    fn files(&self, scope: Scope) -> Vec<PathBuf> {
        let mut result = vec![
            self.config.join("preferences.json"),
            self.config.join("control.json"),
            self.binary.clone(),
            self.definition.clone(),
        ];
        if scope == Scope::Machine {
            result.extend([
                self.secrets.join("login.cred"),
                self.secrets.join("login.json"),
            ]);
        }
        result
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    security: Option<String>,
}
impl Drop for File {
    fn drop(&mut self) {
        if let Some(bytes) = &mut self.bytes {
            bytes.fill(0);
        }
    }
}
impl File {
    fn capture(path: PathBuf) -> Result<Self> {
        win::plain_path(&path)?;
        match fs::read(&path) {
            Ok(bytes) => {
                let security = Some(win::security(&path)?);
                Ok(Self {
                    path,
                    bytes: Some(bytes),
                    security,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path,
                bytes: None,
                security: None,
            }),
            Err(error) => Err(error.into()),
        }
    }
    fn restore(&self) -> Result<()> {
        if let Some(bytes) = &self.bytes {
            win::write_atomic(&self.path, bytes)?;
            if let Some(security) = &self.security {
                win::set_security(&self.path, security)?;
            }
        } else {
            remove_file(&self.path)?;
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Directory {
    path: PathBuf,
    security: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    version: u32,
    scope: Scope,
    sid: String,
    files: Vec<File>,
    directories: Vec<Directory>,
    locks: Vec<(PathBuf, bool)>,
    #[serde(default)]
    control_invalid: bool,
    credential: Option<Credential>,
    pub registration: Option<Registration>,
    pub generation: u64,
    pub maximum_generation: u64,
}
impl Snapshot {
    pub fn capture(paths: &Paths, scope: Scope, changes_credentials: bool) -> Result<Self> {
        Self::capture_with(paths, scope, changes_credentials, false)
    }
    pub fn capture_for_removal(paths: &Paths, scope: Scope) -> Result<Self> {
        Self::capture_with(paths, scope, false, true)
    }
    fn capture_with(
        paths: &Paths,
        scope: Scope,
        changes_credentials: bool,
        allow_invalid_control: bool,
    ) -> Result<Self> {
        let directory_roots = vec![
            paths.config.clone(),
            paths.secrets.clone(),
            paths
                .binary
                .parent()
                .context("Binary has no parent")?
                .to_path_buf(),
            paths
                .definition
                .parent()
                .context("Definition has no parent")?
                .to_path_buf(),
        ];
        let mut directories = Vec::new();
        for root in directory_roots {
            for path in root.ancestors() {
                if path
                    .file_name()
                    .is_some_and(|name| name == identity::ORGANIZATION)
                {
                    break;
                }
                directories.push(path.to_path_buf());
            }
        }
        directories.sort();
        directories.dedup();
        let directories = directories
            .into_iter()
            .map(|path| {
                win::plain_path(&path)?;
                let security = if path.try_exists()? {
                    Some(win::security(&path)?)
                } else {
                    None
                };
                Ok(Directory { path, security })
            })
            .collect::<Result<Vec<_>>>()?;
        // The daemon may be running during an update; serialize only service mutation and authentication.
        let mut locks = Vec::new();
        for name in ["service.lock", "operation.lock"] {
            let path = paths.config.join(name);
            win::plain_path(&path)?;
            let lock = match fs::OpenOptions::new().read(true).write(true).open(path) {
                Ok(lock) => lock,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            fs2::FileExt::try_lock_exclusive(&lock)
                .context("Application state is busy during snapshot")?;
            locks.push(lock);
        }
        let files = paths
            .files(scope)
            .into_iter()
            .map(File::capture)
            .collect::<Result<Vec<_>>>()?;
        let credential = if scope == Scope::User {
            credentials::capture()?
        } else {
            None
        };
        let registration = services::capture(scope)?;
        // Validate the on-disk ownership marker before treating a native registration as ours.
        if scope == Scope::Machine && registration.is_some() {
            let definition: serde_json::Value =
                serde_json::from_slice(&fs::read(&paths.definition)?)?;
            ensure!(
                definition["managed_by"] == identity::APP_ID && definition["scope"] == "system",
                "Service ownership marker mismatch"
            );
        }
        let control_invalid = files
            .iter()
            .find(|file| file.path == paths.config.join("control.json"))
            .and_then(|file| file.bytes.as_ref())
            .is_some_and(|bytes| {
                serde_json::from_slice::<serde_json::Value>(bytes)
                    .map_or(true, |value| !value.is_object())
            });
        let generation = if control_invalid && allow_invalid_control {
            0
        } else {
            generation(paths)?
        };
        let lock_files = ["service.lock", "operation.lock", "daemon.lock"]
            .into_iter()
            .map(|name| {
                let path = paths.config.join(name);
                let exists = path.try_exists()?;
                Ok((path, exists))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            version: 1,
            scope,
            sid: win::sid()?,
            files,
            directories,
            locks: lock_files,
            control_invalid,
            credential,
            registration,
            generation,
            maximum_generation: generation
                .checked_add(u64::from(changes_credentials))
                .context("Credential generation exhausted")?,
        })
    }
    pub fn fresh(&self) -> bool {
        self.registration.is_none()
            && self.credential.is_none()
            && self.files.iter().all(|file| file.bytes.is_none())
    }
    pub fn validate(&self, paths: &Paths, scope: Scope) -> Result<()> {
        ensure!(
            self.version == 1
                && self.scope == scope
                && (scope == Scope::Machine || self.sid == win::sid()?),
            "Snapshot scope or identity mismatch"
        );
        ensure!(
            self.files
                .iter()
                .map(|file| &file.path)
                .eq(paths.files(scope).iter()),
            "Snapshot contains unexpected file paths"
        );
        Ok(())
    }
    pub fn restore_files(&self, paths: &Paths) -> Result<()> {
        let current_generation = self.current_generation(paths)?;
        ensure!(
            current_generation <= self.maximum_generation,
            "Credentials changed concurrently; snapshot retained for recovery"
        );
        // The caller has stopped the daemon and holds both daemon and operation locks.
        let restore_generation = !self.fresh() && !self.control_invalid;
        if restore_generation {
            // Restoring credentials must invalidate every permit issued before rollback.
            let control = paths.config.join("control.json");
            let original = self
                .files
                .iter()
                .find(|file| file.path == control)
                .and_then(|file| file.bytes.as_ref());
            let mut state: serde_json::Value = match original {
                Some(bytes) => serde_json::from_slice(bytes)?,
                None => {
                    serde_json::json!({"paused":false,"session":null,"revision":0,"credential_generation":0})
                }
            };
            state["credential_generation"] = serde_json::json!(
                current_generation
                    .max(self.generation)
                    .checked_add(1)
                    .context("Credential generation exhausted")?
            );
            win::write_atomic(&control, &serde_json::to_vec(&state)?)?;
        }
        for file in &self.files {
            if restore_generation && file.path == paths.config.join("control.json") {
                continue;
            }
            file.restore()?;
        }
        if self.scope == Scope::User {
            credentials::restore(self.credential.as_ref())?;
        }
        Ok(())
    }
    pub fn current_generation(&self, paths: &Paths) -> Result<u64> {
        match generation(paths) {
            Err(_) if self.control_invalid => Ok(0),
            value => value,
        }
    }
    pub fn restore_directories(&self) -> Result<()> {
        if !self
            .registration
            .as_ref()
            .is_some_and(|registration| registration.running)
        {
            for (path, existed) in &self.locks {
                if !existed {
                    remove_file(path)?;
                }
            }
        }
        let mut directories: Vec<_> = self.directories.iter().collect();
        directories.sort_by_key(|directory| std::cmp::Reverse(directory.path.components().count()));
        for directory in directories {
            if let Some(security) = &directory.security {
                fs::create_dir_all(&directory.path)?;
                win::set_security(&directory.path, security)?;
            } else if directory.path.try_exists()? {
                // Only empty application directories created by this operation may be pruned.
                match fs::remove_dir(&directory.path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }
}
pub fn read_control(paths: &Paths) -> Result<serde_json::Value> {
    match fs::read(paths.config.join("control.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid control state"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(
            serde_json::json!({"paused":false,"session":null,"revision":0,"credential_generation":0}),
        ),
        Err(error) => Err(error.into()),
    }
}
pub fn generation(paths: &Paths) -> Result<u64> {
    Ok(read_control(paths)?["credential_generation"]
        .as_u64()
        .unwrap_or(0))
}
pub fn remove_file(path: &Path) -> Result<()> {
    win::plain_path(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_paths_work_without_an_interactive_user_profile() -> Result<()> {
        let paths = Paths::resolve(Scope::Machine, || {
            anyhow::bail!("No interactive Windows profile")
        })?;
        assert!(paths.config.starts_with(win::program_data()?));
        assert!(paths.install.starts_with(win::program_files()?));
        assert!(
            Paths::resolve(Scope::User, || anyhow::bail!(
                "No interactive Windows profile"
            ))
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn backup_restores_existing_files_and_removes_new_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("preferences.json");
        let absent = File::capture(path.clone())?;
        fs::write(&path, b"before")?;
        let original = File::capture(path.clone())?;
        fs::write(&path, b"after")?;
        original.restore()?;
        assert_eq!(fs::read(&path)?, b"before");
        absent.restore()?;
        assert!(!path.exists());
        Ok(())
    }

    fn fixture() -> Result<(tempfile::TempDir, Paths, Snapshot)> {
        let directory = tempfile::tempdir()?;
        let paths = Paths {
            config: directory.path().join("state"),
            binary: directory.path().join("bin/daemon.exe"),
            definition: directory.path().join("service.json"),
            secrets: directory.path().join("secrets"),
            install: directory.path().join("program"),
        };
        fs::create_dir_all(&paths.config)?;
        fs::write(
            paths.config.join("preferences.json"),
            br#"{"line":"free2"}"#,
        )?;
        fs::write(
            paths.config.join("control.json"),
            br#"{"paused":true,"session":null,"revision":4,"credential_generation":8}"#,
        )?;
        let files = paths
            .files(Scope::Machine)
            .into_iter()
            .map(File::capture)
            .collect::<Result<Vec<_>>>()?;
        let snapshot = Snapshot {
            version: 1,
            scope: Scope::Machine,
            sid: "test".into(),
            files,
            directories: Vec::new(),
            locks: Vec::new(),
            control_invalid: false,
            credential: None,
            registration: None,
            generation: 8,
            maximum_generation: 9,
        };
        Ok((directory, paths, snapshot))
    }

    #[test]
    fn old_data_without_a_service_is_preserved_and_permits_are_invalidated() -> Result<()> {
        let (_directory, paths, snapshot) = fixture()?;
        assert!(!snapshot.fresh());
        fs::write(
            paths.config.join("preferences.json"),
            br#"{"line":"free1"}"#,
        )?;
        fs::write(
            paths.config.join("control.json"),
            br#"{"paused":false,"session":null,"revision":5,"credential_generation":9}"#,
        )?;
        snapshot.restore_files(&paths)?;
        let state = read_control(&paths)?;
        assert_eq!(state["paused"], true);
        assert_eq!(state["credential_generation"], 10);
        assert_eq!(
            fs::read(paths.config.join("preferences.json"))?,
            br#"{"line":"free2"}"#
        );
        Ok(())
    }

    #[test]
    fn concurrent_credential_change_prevents_overwriting_current_data() -> Result<()> {
        let (_directory, paths, snapshot) = fixture()?;
        let current = br#"{"paused":false,"session":null,"revision":6,"credential_generation":10}"#;
        fs::write(paths.config.join("control.json"), current)?;
        fs::write(
            paths.config.join("preferences.json"),
            b"concurrent preferences",
        )?;
        assert!(snapshot.restore_files(&paths).is_err());
        assert_eq!(fs::read(paths.config.join("control.json"))?, current);
        assert_eq!(
            fs::read(paths.config.join("preferences.json"))?,
            b"concurrent preferences"
        );
        Ok(())
    }
    #[test]
    fn a_failed_purge_can_restore_corrupt_control_data() -> Result<()> {
        let (_directory, paths, mut snapshot) = fixture()?;
        let control = paths.config.join("control.json");
        let original = b"previous corrupt control data";
        snapshot
            .files
            .iter_mut()
            .find(|file| file.path == control)
            .expect("control backup")
            .bytes = Some(original.to_vec());
        snapshot.control_invalid = true;
        fs::remove_file(&control)?;
        snapshot.restore_files(&paths)?;
        assert_eq!(fs::read(&control)?, original);
        Ok(())
    }
}
