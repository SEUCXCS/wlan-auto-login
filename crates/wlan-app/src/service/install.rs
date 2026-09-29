//! Install or remove service artifacts, restoring previous files on failure.
use super::definition::{Plan, assert_owned, ensure_no_conflicting_install, plan};
#[cfg(windows)]
use super::{command::SystemRunner, lifecycle};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use crate::{
    infra::control,
    infra::{filesystem::private_dir, paths::Paths},
};
use std::{fs, path::Path};

pub async fn install(paths: &Paths) -> Result<()> {
    ensure_no_conflicting_install(paths)?;
    let plan = plan(paths)?;
    #[cfg(target_os = "linux")]
    let plan = {
        let mut plan = plan;
        if paths.scope == crate::infra::scope::Scope::System {
            plan.content = plan.content.replace(
                "# CREDENTIALS\n",
                &crate::credentials::linux_credential_directive(paths)?,
            );
        }
        plan
    };
    install_plan(paths, plan, &std::env::current_exe()?).await
}

async fn install_plan(paths: &Paths, plan: Plan, source: &Path) -> Result<()> {
    crate::infra::error::ensure!(
        plan.scope == paths.scope,
        Message::ServicePlanDoesNotMatchTheSelected
    );
    let _service_lock = control::service_lock(paths)?;
    assert_owned(&plan.definition)?;
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
    stop_before_replacing(paths, &plan).await?;
    let outcome: Result<()> = async {
        if source != plan.executable.canonicalize().unwrap_or_default() {
            staged.persist(&plan.executable).map_err(|e| e.error)?;
        }
        #[cfg(target_os = "macos")]
        if paths.scope == crate::infra::scope::Scope::System {
            crate::credentials::authorize_binary(&plan.executable)?;
        }
        write_definition(&plan.definition, plan.content.as_bytes())?;
        for command in &plan.start {
            command.run().await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        // Attempt every rollback step, and report incomplete recovery accurately.
        let mut rollback_errors = Vec::new();
        if let Err(e) = stop_before_replacing(paths, &plan).await {
            rollback_errors.push(e.to_string());
        }
        for command in &plan.remove {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        for result in [
            old_executable.restore(&plan.executable),
            old_definition.restore(&plan.definition),
        ] {
            if let Err(e) = result {
                rollback_errors.push(e.to_string());
            }
        }
        #[cfg(target_os = "macos")]
        if paths.scope == crate::infra::scope::Scope::System
            && old_executable.existed()
            && let Err(e) = crate::credentials::authorize_binary(&plan.executable)
        {
            rollback_errors.push(e.to_string());
        }
        let commands = if old_definition.existed() {
            &plan.start
        } else {
            &plan.reload
        };
        for command in commands {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            return Err(error).context(
                (Message::InstallationFailedAndRollbackWasIncomplete {
                    value0: (rollback_errors.join("；")).to_string(),
                })
                .to_string(),
            );
        }
        return Err(error).context(Message::InstallationFailedInstallationFilesRestored);
    }
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

async fn stop_before_replacing(paths: &Paths, plan: &Plan) -> Result<()> {
    for command in &plan.stop {
        command.run().await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    #[cfg(windows)]
    lifecycle::wait_task_stopped(&plan.manager, &SystemRunner).await?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
// Service directories may already contain other applications' files; do not chmod them.
fn write_definition(path: &Path, bytes: &[u8]) -> Result<()> {
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
struct Backup(Option<tempfile::NamedTempFile>);
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
    use crate::service::{command::Invocation, definition::LABEL, manager::Manager};

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
