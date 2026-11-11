//! Unregister services and optionally remove all data owned by the selected scope.
use crate::infra::command::{Invocation, Runner, SystemRunner};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, ensure};
use crate::system::define::services::Manager;
use crate::system::define::services::ServiceManager;
use crate::system::shared::services::artifacts::{Plan, assert_owned, plan};
use crate::{infra::cleanup, infra::control, infra::paths::Paths};
use std::{future::Future, time::Duration};

pub async fn run(paths: &Paths, purge: bool) -> Result<()> {
    run_with(
        paths,
        purge,
        &crate::system::shared::services::hooks::DefaultHooks,
    )
    .await
}

pub(crate) async fn run_with(
    paths: &Paths,
    purge: bool,
    hooks: &impl crate::system::shared::services::hooks::UninstallHooks,
) -> Result<()> {
    // The native override supplies extra steps; the transaction order remains common.
    uninstall(paths, &plan(paths)?, purge, &SystemRunner, hooks).await?;

    if purge {
        tracing::info!(
            "{}",
            Message::ServiceAndApplicationDataRemovedForScope {
                value0: (paths.scope.label()).to_string()
            }
        );
        tracing::info!("{}", Message::SharedSystemLogsAndServiceAuditRecords);
    } else {
        tracing::info!("{}", Message::ServiceUninstalledSavedCredentialsRetained);
    }
    Ok(())
}

async fn uninstall(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    hooks: &impl crate::system::shared::services::hooks::UninstallHooks,
) -> Result<()> {
    // The storage future is polled only after verified stop/unregistration and
    // while daemon and operation locks are held. No background read can survive
    // that barrier, so purge does not need to parse the state it will remove.
    uninstall_with(
        paths,
        plan,
        purge,
        runner,
        crate::system::shared::credentials::delete(paths),
        hooks,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn uninstall_plan(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
) -> Result<()> {
    uninstall_with(
        paths,
        plan,
        purge,
        runner,
        delete_credentials,
        &crate::system::shared::flow::tests::FixtureHooks,
    )
    .await
}

async fn uninstall_with(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
    hooks: &impl crate::system::shared::services::hooks::UninstallHooks,
) -> Result<()> {
    // Validate scope, directory boundaries and ownership before any mutation.
    ensure!(
        plan.scope == paths.scope,
        Message::ServicePlanDoesNotMatchTheSelected
    );
    for directory in [&paths.config, &paths.runtime, &paths.secrets] {
        cleanup::validate_directory(directory)?;
    }
    assert_owned(&plan.definition)?;
    hooks.validate(plan).await?;

    // Serialize service mutation, then inspect registration independently of files.
    let service_lock = control::service_lock(paths)?;
    // Query registrations even when the definition was manually removed.
    let state = plan.manager.query(runner).await?;
    if hooks.must_stop(plan, &state) {
        // Stop native registration before waiting or touching installed artifacts.
        hooks.stop_registered(plan, runner).await?;
        wait_for_manager(&plan.manager, runner, false).await?;
    }

    // Stop authenticated processes and native helpers before taking deletion locks.
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    hooks.stop_helpers(paths, plan).await?;

    // Hold daemon and operation locks across unregistration and optional data deletion.
    let daemon_lock = control::singleton(paths)?;
    let operation_lock = control::operation_lock(paths).await?;
    if state.registered {
        for command in &plan.remove {
            execute(runner, command).await?;
        }
    }
    remove_enable_link(plan)?;
    // Signed definitions remain inside their bundle until credential cleanup succeeds.
    if !hooks.packaged_definition(plan) {
        cleanup::remove_file(&plan.definition)?;
    }
    for command in &plan.reload {
        execute(runner, command).await?;
    }
    wait_for_manager(&plan.manager, runner, true).await?;

    if purge {
        delete_credentials
            .await
            .context(Message::ServiceUninstalledButCredentialDeletionFailedRetryUninstall)?;
    }
    // Remove the native payload only after confirmed unregistration and credential deletion.
    hooks.remove_payload(plan)?;

    if purge {
        for name in ["preferences.json", "control.json", "daemon.log"] {
            cleanup::remove_file(&paths.config.join(name))?;
        }
        for name in ["daemon-endpoint.json", "daemon.sock"] {
            cleanup::remove_file(&paths.runtime.join(name))?;
        }
        // Windows requires the handles closed before their lock files are removed.
        drop(operation_lock);
        drop(daemon_lock);
        for name in hooks.runtime_files(paths, plan) {
            cleanup::remove_file(&paths.runtime.join(name))?;
        }
        for name in ["operation.lock", "daemon.lock"] {
            cleanup::remove_file(&paths.runtime.join(name))?;
        }
        drop(service_lock);
        cleanup::remove_file(&paths.config.join("service.lock"))?;
        for directory in [&paths.runtime, &paths.secrets, &paths.config] {
            cleanup::prune_empty_directories(directory)?;
        }
        cleanup::prune_empty_directories(
            plan.executable
                .parent()
                .context(Message::InstallationPathHasNoParentDirectory)?,
        )?;
    }
    Ok(())
}

fn remove_enable_link(plan: &Plan) -> Result<()> {
    if let Manager::Systemd { system } = &plan.manager {
        let parent = plan
            .definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?;
        let name = plan
            .definition
            .file_name()
            .context(Message::ServiceDefinitionHasNoFileName)?;
        let path = parent
            .join(if *system {
                "multi-user.target.wants"
            } else {
                "default.target.wants"
            })
            .join(name);
        cleanup::validate_directory(
            path.parent()
                .context(Message::ServiceLinkHasNoParentDirectory)?,
        )?;
        match std::fs::read_link(&path) {
            Ok(target) => {
                ensure!(
                    target == plan.definition || target == std::path::Path::new("..").join(name),
                    Message::RefusingToRemoveAnAutostartLinkOwned {
                        value0: (path.display()).to_string()
                    }
                );
                cleanup::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context(Message::FailedToRemoveServiceAutostartLink);
            }
        }
    }

    Ok(())
}

async fn execute(runner: &impl Runner, command: &Invocation) -> Result<()> {
    crate::system::shared::services::steps::execute_optional(runner, command).await
}

async fn wait_for_manager(manager: &Manager, runner: &impl Runner, removed: bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let state = manager.query(runner).await?;
            if !state.running && !state.pending && (!removed || !state.registered) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context(if removed {
        "Timed out waiting for service removal; application data retained"
    } else {
        "Timed out waiting for service stop; installation files retained"
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::{FakeRunner, STOPPED};
    use crate::system::shared::flow::tests::{FixtureHooks, fixture, removal_runner};
    use std::fs;

    #[tokio::test]
    async fn purge_deletes_scoped_storage_without_parsing_control_state() -> Result<()> {
        for state in [
            r#"{"paused":false,"session":null,"revision":0}"#,
            "invalid json",
            r#"{"paused":false,"session":null,"revision":0,"credential_generation":18446744073709551615}"#,
        ] {
            for already_unregistered in [false, true] {
                let dir = tempfile::tempdir()?;
                let (paths, plan) = fixture(dir.path(), 1)?;
                fs::write(paths.config.join("control.json"), state)?;
                fs::write(paths.secrets.join("login.json"), "invalid credential data")?;
                let sibling = dir.path().join("SEUCXCS/another-tool");
                fs::create_dir(&sibling)?;
                fs::write(sibling.join("settings.json"), "preserve")?;
                let runner = if already_unregistered {
                    fs::remove_file(&plan.definition)?;
                    FakeRunner::new([
                        (true, "LoadState=not-found\n"),
                        (true, ""),
                        (true, "LoadState=not-found\n"),
                    ])
                } else {
                    removal_runner(1)
                };

                // Exercise the production storage selection with temporary system
                // files; service-manager operations remain entirely simulated.
                uninstall(&paths, &plan, true, &runner, &FixtureHooks).await?;

                assert!(!plan.definition.exists() && !plan.executable.exists());
                assert!(!paths.config.exists() && !paths.runtime.exists());
                assert!(!paths.secrets.exists());
                assert!(!dir.path().join("SEUCXCS/wlanlogin").exists());
                assert_eq!(
                    fs::read_to_string(sibling.join("settings.json"))?,
                    "preserve"
                );
                assert!(dir.path().join("original-cli").exists());
                assert!(runner.outputs.borrow().is_empty());
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_unregister_never_polls_scoped_credential_deletion() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 1)?;
        let state = "invalid json";
        fs::write(paths.config.join("control.json"), state)?;
        let runner = FakeRunner::new([(true, STOPPED), (false, "permission denied")]);

        assert!(
            uninstall(&paths, &plan, true, &runner, &FixtureHooks)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(paths.secrets.join("login.cred"))?,
            "encrypted test secret"
        );
        assert_eq!(
            fs::read_to_string(paths.config.join("control.json"))?,
            state
        );
        assert!(plan.definition.exists() && plan.executable.exists());
        Ok(())
    }

    #[tokio::test]
    async fn purge_removes_only_owned_dangling_systemd_enable_links() -> Result<()> {
        for (kind, directory) in [(0, "default.target.wants"), (1, "multi-user.target.wants")] {
            let dir = tempfile::tempdir()?;
            let (paths, plan) = fixture(dir.path(), kind)?;
            let wants = plan
                .definition
                .parent()
                .context("definition parent")?
                .join(directory);
            fs::create_dir(&wants)?;
            let link = wants.join("service");
            fs::remove_file(&plan.definition)?;
            std::os::unix::fs::symlink("../service", &link)?;
            let runner = FakeRunner::new([
                (true, "LoadState=not-found\n"),
                (true, ""),
                (true, "LoadState=not-found\n"),
            ]);
            uninstall_plan(&paths, &plan, true, &runner, async {
                cleanup::remove_file(&paths.secrets.join("login.cred"))
            })
            .await?;
            assert!(fs::symlink_metadata(&link).is_err());

            std::os::unix::fs::symlink("../another-service", &link)?;
            assert!(remove_enable_link(&plan).is_err());
            assert!(fs::symlink_metadata(&link).is_ok());
            assert_eq!(
                fs::read_to_string(wants.join("../another-service"))?,
                "preserve"
            );
        }
        Ok(())
    }
}
