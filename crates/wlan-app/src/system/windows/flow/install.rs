//! Platform-owned installation transaction; native preparation stays here.

use crate::infra::command::{Invocation, Runner, SystemRunner};
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
    install_plan(paths, plan, &std::env::current_exe()?, &SystemRunner).await
}

async fn install_plan(
    paths: &Paths,
    plan: Plan,
    source: &Path,
    runner: &impl Runner,
) -> Result<()> {
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

    // Validate the actual encoded task file before stopping an existing service.
    let definition_bytes = crate::system::windows::services::artifacts::definition_bytes(&plan);
    let staged_definition = stage_definition(&plan.definition, &definition_bytes)?;
    if matches!(
        plan.manager,
        crate::system::define::services::Manager::TaskScheduler { .. }
    ) {
        crate::system::windows::services::task::validate(staged_definition.path(), runner).await?;
    }

    // Registration is independent of stale files left by an earlier failed install.
    let previous_task = match &plan.manager {
        crate::system::define::services::Manager::TaskScheduler { name } => {
            Some(crate::system::windows::services::task::capture(name, runner).await?)
        }
        _ => None,
    };

    // Capture both artifacts before staging the replacement executable.
    let old_definition = Backup::capture(&plan.definition)?;
    let old_executable = Backup::capture(&plan.executable)?;
    // Stage the executable before changing a running service.
    let staged = tempfile::NamedTempFile::new_in(binary_dir)?;
    fs::copy(&source, staged.path())?;

    // Stop and wait before replacing either file.
    stop_before_replacing(paths, &plan, runner).await?;

    // Publish the staged binary and definition, then register and start.
    let outcome: Result<()> = async {
        if source != plan.executable.canonicalize().unwrap_or_default() {
            staged.persist(&plan.executable).map_err(|e| e.error)?;
        }

        staged_definition
            .persist(&plan.definition)
            .map_err(|e| e.error)?;
        for command in &plan.start {
            execute(command, runner).await?;
        }
        if matches!(
            plan.manager,
            crate::system::define::services::Manager::TaskScheduler { .. }
        ) {
            steps::wait_task_started(&plan.manager, runner).await?;
        }
        Ok(())
    }
    .await;

    // Recovery uses the same stop/unregister barrier as signed bundles.
    if let Err(error) = outcome {
        // A failed stop or unregister forbids restoring files under a live process.
        let mut rollback = crate::system::shared::services::transaction::Rollback::default();
        rollback
            .step(stop_before_replacing(paths, &plan, runner))
            .await;
        if let crate::system::define::services::Manager::TaskScheduler { name } = &plan.manager {
            rollback
                .step(crate::system::windows::services::task::remove(name, runner))
                .await;
        } else {
            for command in &plan.remove {
                rollback.step(execute(command, runner)).await;
            }
        }

        if rollback.safe() {
            rollback.record(old_executable.restore(&plan.executable));
            rollback.record(old_definition.restore(&plan.definition));
        }

        if rollback.safe() {
            if let (
                Some(snapshot),
                crate::system::define::services::Manager::TaskScheduler { name },
            ) = (&previous_task, &plan.manager)
            {
                rollback
                    .step(snapshot.restore(name, &plan.definition, runner))
                    .await;
            } else {
                let commands = if old_definition.existed() {
                    &plan.start
                } else {
                    &plan.reload
                };
                for command in commands {
                    rollback.step(execute(command, runner)).await;
                }
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
            if let Some(snapshot) = &previous_task {
                match snapshot.retain(&plan.definition) {
                    Ok(Some(path)) => recovery.push(path.display().to_string()),
                    Ok(None) => {}
                    Err(retain_error) => rollback.record(Err(retain_error)),
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

async fn stop_before_replacing(paths: &Paths, plan: &Plan, runner: &impl Runner) -> Result<()> {
    for command in &plan.stop {
        execute(command, runner).await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;

    steps::wait_task_stopped(&plan.manager, runner).await?;
    Ok(())
}
use crate::system::shared::services::payload::Backup;
fn stage_definition(path: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;

async fn execute(command: &Invocation, runner: &impl Runner) -> Result<()> {
    let result = runner.output(command).await?;
    if !command.optional {
        result.check(command)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::{CommandOutput, tests::FakeRunner};
    const ABSENT: &str = "Installed=0";
    const READY: &str = "Installed=1
State=3
Enabled=1
LastTaskResult=0";
    fn runner(outputs: &[(bool, &str)]) -> FakeRunner {
        let runner = FakeRunner::default();
        runner
            .outputs
            .borrow_mut()
            .extend(outputs.iter().map(|(success, text)| CommandOutput {
                success: *success,
                stdout: (*text).into(),
                stderr: String::new(),
            }));
        runner
    }
    fn fixture(residue: bool) -> Result<(tempfile::TempDir, Paths, Plan, std::path::PathBuf)> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        std::fs::create_dir_all(&paths.config)?;
        let binary = dir.path().join("bin/daemon.exe");
        std::fs::create_dir_all(binary.parent().context("binary parent")?)?;
        let plan = crate::system::windows::services::definition::windows_plan(
            binary,
            paths.config.join("task.xml"),
            "S-1-5-21-123",
        )?;
        if residue {
            std::fs::write(&plan.executable, b"old binary")?;
            std::fs::write(
                &plan.definition,
                format!("{} stale disk definition", crate::infra::identity::APP_ID),
            )?;
        }
        let source = dir.path().join("source.exe");
        std::fs::write(&source, b"new binary")?;
        Ok((dir, paths, plan, source))
    }
    fn command_count(runner: &FakeRunner, verb: &str) -> usize {
        runner
            .calls
            .borrow()
            .iter()
            .filter(|command| command.args.first().is_some_and(|arg| arg == verb))
            .count()
    }
    #[tokio::test]
    async fn invalid_xml_or_snapshot_failure_never_stops_the_old_task() -> Result<()> {
        for outputs in [
            vec![(false, "invalid XML")],
            vec![(true, ""), (false, "query denied")],
        ] {
            let (_dir, paths, plan, source) = fixture(true)?;
            let binary = plan.executable.clone();
            let definition = plan.definition.clone();
            let before = std::fs::read(&definition)?;
            let runner = runner(&outputs);
            assert!(install_plan(&paths, plan, &source, &runner).await.is_err());
            assert_eq!(std::fs::read(binary)?, b"old binary");
            assert_eq!(std::fs::read(definition)?, before);
            assert_eq!(command_count(&runner, "/End"), 0);
        }
        Ok(())
    }
    #[tokio::test]
    async fn failed_registration_with_no_task_rolls_back_even_with_stale_files() -> Result<()> {
        for residue in [false, true] {
            let (_dir, paths, plan, source) = fixture(residue)?;
            let binary = plan.executable.clone();
            let definition = plan.definition.clone();
            let before = std::fs::read(&definition).ok();
            let runner = runner(&[
                (true, ""),
                (true, ABSENT),
                (true, ""),
                (true, ABSENT),
                (false, "registration failed"),
                (true, ""),
                (true, ABSENT),
                (true, ABSENT),
            ]);
            let error = install_plan(&paths, plan, &source, &runner)
                .await
                .err()
                .context("expected failure")?;
            assert!(!error.to_string().contains("rollback was incomplete"));
            assert!(error.to_string().contains("registration failed"));
            assert_eq!(std::fs::read(&definition).ok(), before);
            assert_eq!(binary.exists(), residue);
            assert_eq!(command_count(&runner, "/Delete"), 0);
            assert_eq!(command_count(&runner, "/Create"), 1);
            assert_eq!(command_count(&runner, "/Run"), 0);
        }
        Ok(())
    }
    #[tokio::test]
    async fn failed_start_removes_new_registration_before_restoring_files() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(false)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (false, "start failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
        ]);
        assert!(install_plan(&paths, plan, &source, &runner).await.is_err());
        assert!(!binary.exists());
        assert!(!definition.exists());
        assert_eq!(command_count(&runner, "/Delete"), 1);
        Ok(())
    }
    #[tokio::test]
    async fn failed_unregister_retains_published_files_and_old_backups() -> Result<()> {
        let (dir, paths, plan, source) = fixture(true)?;
        let binary = plan.executable.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (false, "start failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (false, "delete denied"),
            (true, READY),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(error.to_string().contains("recovery files"));
        assert!(error.to_string().contains("delete denied"));
        assert_eq!(std::fs::read(binary)?, b"new binary");
        let backups = std::fs::read_dir(dir.path().join("bin"))?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp"))
            .map(|entry| std::fs::read(entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(backups.iter().any(|bytes| bytes == b"old binary"));
        Ok(())
    }
    #[tokio::test]
    async fn stopped_registered_task_is_restored_without_starting_it() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(true)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let before = std::fs::read(&definition)?;
        let native = plan.content.clone();
        let runner = runner(&[
            (true, ""),
            (true, READY),
            (true, &native),
            (true, ""),
            (true, READY),
            (false, "registration failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, READY),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(!error.to_string().contains("rollback was incomplete"));
        assert_eq!(std::fs::read(binary)?, b"old binary");
        assert_eq!(std::fs::read(definition)?, before);
        assert_eq!(command_count(&runner, "/Run"), 0);
        assert_eq!(command_count(&runner, "/Create"), 2);
        Ok(())
    }
    #[tokio::test]
    async fn successful_install_checks_liveness_and_publishes_utf16() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(false)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ""),
            (
                true,
                "Installed=1
State=4
Enabled=1",
            ),
        ]);
        install_plan(&paths, plan, &source, &runner).await?;
        assert_eq!(std::fs::read(binary)?, b"new binary");
        assert!(std::fs::read(definition)?.starts_with(&[0xff, 0xfe]));
        assert_eq!(command_count(&runner, "/Delete"), 0);
        assert!(runner.outputs.borrow().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn failed_native_restore_retains_the_registration_xml_for_recovery() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(true)?;
        let native = plan
            .content
            .replace("CXCS WLAN Login", "original registered task");
        let runner = runner(&[
            (true, ""),
            (true, READY),
            (true, &native),
            (true, ""),
            (true, READY),
            (false, "registration failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
            (false, "restore denied"),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(error.to_string().contains("restore denied"));
        let recovered = std::fs::read_dir(&paths.config)?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp"))
            .filter_map(|entry| {
                crate::system::windows::services::artifacts::read_definition(&entry.path()).ok()
            })
            .collect::<Vec<_>>();
        assert!(recovered.iter().any(|xml| xml == &native));
        Ok(())
    }
}
