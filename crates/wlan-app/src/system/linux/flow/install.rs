//! Platform-owned installation transaction; native preparation stays here.

use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use crate::infra::scope::Scope;

use crate::system::shared::services::artifacts::{Plan, assert_owned};

use crate::system::shared::services::artifacts::{ensure_no_conflicting_install, plan};

use crate::{
    infra::control,
    infra::{filesystem::private_dir, paths::Paths},
};
use std::{fs, path::Path};

async fn run_with(paths: &Paths, prepare: impl FnOnce(Plan) -> Result<Plan>) -> Result<()> {
    // Validate scope ownership before constructing or changing native artifacts.
    ensure_no_conflicting_install(paths)?;
    let plan = prepare(plan(paths)?)?;

    // Stage and publish through the common transaction.
    install_plan(paths, plan, &std::env::current_exe()?).await
}

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

    if paths.scope == crate::infra::scope::Scope::System {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(binary_dir, fs::Permissions::from_mode(0o755))?;
    }
    if let Some(parent) = plan.definition.parent() {
        fs::create_dir_all(parent)?;
    }

    // Capture both artifacts before staging the replacement executable.
    let old_definition = Backup::capture(&plan.definition)?;
    let old_executable = Backup::capture(&plan.executable)?;
    // Stage the executable before changing a running service.
    let staged = tempfile::NamedTempFile::new_in(binary_dir)?;
    fs::copy(&source, staged.path())?;

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

async fn stop_before_replacing(paths: &Paths, plan: &Plan) -> Result<()> {
    for command in &plan.stop {
        command.run().await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;

    Ok(())
}
use crate::system::shared::services::payload::Backup;
pub(crate) fn write_definition(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;

    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}

pub(crate) async fn run(paths: &Paths) -> Result<()> {
    // System services receive native encrypted credentials through systemd.
    run_with(paths, |mut plan| {
        if paths.scope == Scope::System {
            plan.content = plan.content.replace(
                "# CREDENTIALS\n",
                &crate::system::credentials::linux_credential_directive(paths)?,
            );
        }
        Ok(plan)
    })
    .await
}

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::infra::command::Invocation;

    use crate::system::define::services::{LABEL, Manager};

    #[tokio::test]
    async fn failed_registration_restores_the_previous_service_and_binary() -> Result<()> {
        let scopes = if true {
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

    #[tokio::test]
    async fn failed_first_install_removes_new_artifacts() -> Result<()> {
        let scopes = if true {
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
}
