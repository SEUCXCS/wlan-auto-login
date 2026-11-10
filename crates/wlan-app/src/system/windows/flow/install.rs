//! Platform-owned installation transaction; native preparation stays here.

use crate::infra::command::SystemRunner;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};

use crate::system::shared::services::artifacts::{Plan, assert_owned};

use crate::system::shared::services::artifacts::{ensure_no_conflicting_install, plan};

use crate::system::windows::services::wait as steps;

use crate::{
    infra::control,
    infra::{filesystem::private_dir, paths::Paths},
};
use std::{fs, path::Path};

pub async fn run(paths: &Paths) -> Result<()> {
    run_with(paths, Ok).await
}

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
        crate::infra::filesystem::restrict_windows(binary_dir, true)?;
    }

    if let Some(parent) = plan.definition.parent() {
        fs::create_dir_all(parent)?;

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

    steps::wait_task_stopped(&plan.manager, &SystemRunner).await?;
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

    Ok(())
}

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;
