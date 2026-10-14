//! Install or remove service artifacts, restoring previous files on failure.
#[cfg(windows)]
use crate::infra::command::SystemRunner;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
#[cfg(any(not(target_os = "macos"), test))]
use crate::system::shared::services::artifacts::{Plan, assert_owned};
#[cfg(not(target_os = "macos"))]
use crate::system::shared::services::artifacts::{ensure_no_conflicting_install, plan};
#[cfg(windows)]
use crate::system::shared::services::steps;
#[cfg(any(not(target_os = "macos"), test))]
use crate::{
    infra::control,
    infra::{filesystem::private_dir, paths::Paths},
};
use std::{fs, path::Path};

#[cfg(target_os = "windows")]
pub async fn run(paths: &Paths) -> Result<()> {
    run_with(paths, Ok).await
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn run_with(
    paths: &Paths,
    prepare: impl FnOnce(Plan) -> Result<Plan>,
) -> Result<()> {
    // Validate scope ownership before constructing or changing native artifacts.
    ensure_no_conflicting_install(paths)?;
    let plan = prepare(plan(paths)?)?;

    // Stage and publish through the common transaction.
    install_plan(paths, plan, &std::env::current_exe()?).await
}

#[cfg(any(not(target_os = "macos"), test))]
async fn install_plan(paths: &Paths, plan: Plan, source: &Path) -> Result<()> {
    crate::infra::error::ensure!(
        plan.scope == paths.scope,
        Message::ServicePlanDoesNotMatchTheSelected
    );

    // Serialize updates and validate the exact owned definition before staging.
    let _service_lock = control::service_lock(paths)?;
    assert_owned(&plan.definition)?;

    // Prepare scope-appropriate directories without changing shared service folders.
    let source = source.canonicalize()?;
    let binary_dir = plan
        .executable
        .parent()
        .context(Message::InstallationPathHasNoParentDirectory)?;
    private_dir(binary_dir)?;
    #[cfg(windows)]
    if paths.scope == crate::infra::scope::Scope::System {
        crate::infra::filesystem::restrict_windows(binary_dir, true)?;
    }
    #[cfg(unix)]
    if paths.scope == crate::infra::scope::Scope::System {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(binary_dir, fs::Permissions::from_mode(0o755))?;
    }
    if let Some(parent) = plan.definition.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(windows)]
        if paths.scope == crate::infra::scope::Scope::System {
            crate::infra::filesystem::restrict_windows(parent, true)?;
        }
    }

    // Capture both artifacts before staging the replacement executable.
    let old_definition = Backup::capture(&plan.definition)?;
    let old_executable = Backup::capture(&plan.executable)?;
    // Stage the executable before changing a running service.
    let staged = tempfile::NamedTempFile::new_in(binary_dir)?;
    fs::copy(&source, staged.path())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            staged.path(),
            fs::Permissions::from_mode(if paths.scope == crate::infra::scope::Scope::System {
                0o755
            } else {
                0o700
            }),
        )?;
    }

    // Stop and wait before replacing either file.
    stop_before_replacing(paths, &plan).await?;

    // Publish the staged binary and definition, then register and start.
    let outcome: Result<()> = async {
        if source != plan.executable.canonicalize().unwrap_or_default() {
            staged.persist(&plan.executable).map_err(|e| e.error)?;
        }

        write_definition(&plan.definition, plan.content.as_bytes())?;
        for command in &plan.start {
            command.run().await?;
        }
        Ok(())
    }
    .await;

    // Recovery uses the same stop/unregister barrier as signed bundles.
    if let Err(error) = outcome {
        // A failed stop or unregister forbids restoring files under a live process.
        let mut rollback = crate::system::shared::services::transaction::Rollback::default();
        rollback.step(stop_before_replacing(paths, &plan)).await;
        for command in &plan.remove {
            rollback.step(command.run()).await;
        }

        if rollback.safe() {
            rollback.record(old_executable.restore(&plan.executable));
            rollback.record(old_definition.restore(&plan.definition));
        }

        if rollback.safe() {
            let commands = if old_definition.existed() {
                &plan.start
            } else {
                &plan.reload
            };
            for command in commands {
                rollback.step(command.run()).await;
            }
        }

        if !rollback.safe() {
            // Retain every surviving backup for a concrete recovery path.
            let mut recovery = Vec::new();
            for backup in [old_executable, old_definition] {
                if let Some(path) = backup.retain()? {
                    recovery.push(path.display().to_string());
                }
            }
            return Err(error).context(Message::InstallationFailedAndRollbackWasIncomplete {
                value0: format!(
                    "{}; recovery files: {}",
                    rollback.detail(),
                    recovery.join(", ")
                ),
            });
        }
        return Err(error).context(Message::InstallationFailedInstallationFilesRestored);
    }

    // Successful publication is reported only after registration/start checks.
    tracing::info!(
        "{}",
        Message::ServiceInstalledAndStartedForScope {
            value0: (paths.scope.label()).to_string(),
            value1: (if paths.scope == crate::infra::scope::Scope::System {
                "autostart enabled at boot"
            } else {
                "autostart enabled at user login"
            })
            .to_string()
        }
    );
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
async fn stop_before_replacing(paths: &Paths, plan: &Plan) -> Result<()> {
    for command in &plan.stop {
        command.run().await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    #[cfg(windows)]
    steps::wait_task_stopped(&plan.manager, &SystemRunner).await?;
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
// Service directories may already contain other applications' files; do not chmod them.
pub(crate) fn write_definition(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}
#[cfg(any(not(target_os = "macos"), test))]
struct Backup(Option<tempfile::NamedTempFile>);
#[cfg(any(not(target_os = "macos"), test))]
impl Backup {
    fn capture(path: &Path) -> Result<Self> {
        match fs::metadata(path) {
            Ok(_) => {
                let backup = tempfile::NamedTempFile::new_in(
                    path.parent()
                        .context(Message::BackupPathHasNoParentDirectory)?,
                )?;
                fs::copy(path, backup.path())?;
                Ok(Self(Some(backup)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self(None)),
            Err(e) => Err(e.into()),
        }
    }
    fn existed(&self) -> bool {
        self.0.is_some()
    }
    fn retain(mut self) -> Result<Option<std::path::PathBuf>> {
        self.0
            .take()
            .map(|backup| {
                backup
                    .keep()
                    .map(|(_, path)| path)
                    .map_err(|e| e.error.into())
            })
            .transpose()
    }
    fn restore(&self, path: &Path) -> Result<()> {
        match &self.0 {
            Some(backup) => {
                fs::copy(backup.path(), path)?;
                Ok(())
            }
            None => remove_if_present(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::infra::command::Invocation;
    #[cfg(unix)]
    use crate::system::define::services::{LABEL, Manager};

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_registration_restores_the_previous_service_and_binary() -> Result<()> {
        let scopes = if cfg!(target_os = "linux") {
            vec![
                crate::infra::scope::Scope::User,
                crate::infra::scope::Scope::System,
            ]
        } else {
            vec![crate::infra::scope::Scope::User]
        };
        for scope in scopes {
            let dir = tempfile::tempdir()?;
            let mut paths = Paths::test(dir.path().join("config"));
            paths.scope = scope;
            let executable = dir.path().join("daemon");
            let definition = dir.path().join("service");
            let original = dir.path().join("expected");
            let source = dir.path().join("new-binary");
            fs::write(&executable, "old executable")?;
            fs::write(&definition, format!("# {LABEL} old service"))?;
            fs::copy(&definition, &original)?;
            fs::write(&source, "new executable")?;
            let plan = Plan {
                scope,
                manager: Manager::Systemd {
                    system: scope == crate::infra::scope::Scope::System,
                },
                executable: executable.clone(),
                definition: definition.clone(),
                content: format!("# {LABEL} new service"),
                stop: vec![],
                remove: vec![],
                reload: vec![],
                // Fails for the new definition, succeeds after the old one is restored.
                start: vec![Invocation::new(
                    "cmp",
                    [original.as_os_str(), definition.as_os_str()],
                )],
            };
            assert!(install_plan(&paths, plan, &source).await.is_err());
            assert_eq!(fs::read_to_string(executable)?, "old executable");
            assert_eq!(fs::read(definition)?, fs::read(original)?);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_first_install_removes_new_artifacts() -> Result<()> {
        let scopes = if cfg!(target_os = "linux") {
            vec![
                crate::infra::scope::Scope::User,
                crate::infra::scope::Scope::System,
            ]
        } else {
            vec![crate::infra::scope::Scope::User]
        };
        for scope in scopes {
            let dir = tempfile::tempdir()?;
            let mut paths = Paths::test(dir.path().join("config"));
            let source = dir.path().join("source");
            fs::write(&source, "test binary, never executed")?;
            paths.scope = scope;
            let executable = dir.path().join("bin/daemon");
            let definition = dir.path().join("service");
            let plan = Plan {
                scope,
                manager: Manager::Systemd {
                    system: scope == crate::infra::scope::Scope::System,
                },
                executable: executable.clone(),
                definition: definition.clone(),
                content: format!("# {LABEL}"),
                stop: vec![],
                remove: vec![],
                reload: vec![],
                start: vec![Invocation::new("false", std::iter::empty::<&str>())],
            };
            assert!(install_plan(&paths, plan, &source).await.is_err());
            assert!(!executable.exists());
            assert!(!definition.exists());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_rollback_stop_retains_new_files_and_previous_backups() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("state"));
        let executable = dir.path().join("daemon");
        let definition = dir.path().join("service");
        let expected = dir.path().join("old-definition");
        let source = dir.path().join("new-source");
        fs::write(&executable, "old binary")?;
        fs::write(&definition, format!("# {LABEL} old"))?;
        fs::copy(&definition, &expected)?;
        fs::write(&source, "new binary")?;
        let plan = Plan {
            scope: paths.scope,
            manager: Manager::Systemd { system: false },
            executable: executable.clone(),
            definition: definition.clone(),
            content: format!("# {LABEL} new"),
            // The first stop succeeds; after publication, stopping fails.
            stop: vec![Invocation::new(
                "cmp",
                [expected.as_os_str(), definition.as_os_str()],
            )],
            start: vec![Invocation::new("false", std::iter::empty::<&str>())],
            remove: Vec::new(),
            reload: Vec::new(),
        };
        let error = install_plan(&paths, plan, &source)
            .await
            .err()
            .context("expected failed rollback")?;
        assert!(error.to_string().contains("recovery files"));
        assert_eq!(fs::read_to_string(executable)?, "new binary");
        assert_eq!(fs::read_to_string(definition)?, format!("# {LABEL} new"));
        let backups = fs::read_dir(dir.path())?
            .filter_map(std::result::Result::ok)
            .filter(|file| file.file_name().to_string_lossy().starts_with(".tmp"))
            .map(|file| fs::read_to_string(file.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(backups.iter().any(|data| data == "old binary"));
        assert!(backups.iter().any(|data| data == &format!("# {LABEL} old")));
        Ok(())
    }

    #[test]
    fn backups_restore_existing_files_and_remove_new_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("file");
        let absent = Backup::capture(&file)?;
        fs::write(&file, "original")?;
        let backup = Backup::capture(&file)?;
        fs::write(&file, "replacement")?;
        backup.restore(&file)?;
        assert_eq!(fs::read_to_string(&file)?, "original");
        absent.restore(&file)?;
        assert!(!file.exists());
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;
