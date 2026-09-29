//! Unregister services and optionally remove all data owned by the selected scope.
use super::{
    Manager,
    artifacts::{Plan, assert_owned, plan},
};
use crate::infra::command::{Invocation, Runner, SystemRunner};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, ensure};
use crate::system::define::services::ServiceManager;
use crate::{infra::cleanup, infra::control, infra::paths::Paths};
use std::{future::Future, time::Duration};

pub async fn uninstall(
    paths: &Paths,
    purge: bool,
    delete_credentials: impl Future<Output = Result<()>>,
) -> Result<()> {
    uninstall_plan(
        paths,
        &plan(paths)?,
        purge,
        &SystemRunner,
        delete_credentials,
    )
    .await?;
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

async fn uninstall_plan(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
) -> Result<()> {
    ensure!(
        plan.scope == paths.scope,
        Message::ServicePlanDoesNotMatchTheSelected
    );
    for directory in [&paths.config, &paths.runtime, &paths.secrets] {
        cleanup::validate_directory(directory)?;
    }
    assert_owned(&plan.definition)?;
    let service_lock = control::service_lock(paths)?;
    // Query registrations even when the definition was manually removed.
    let state = plan.manager.query(runner).await?;
    if state.running || state.pending {
        for command in &plan.stop {
            execute(runner, command).await?;
        }
        wait_for_manager(&plan.manager, runner, false).await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    let daemon_lock = control::singleton(paths)?;
    let operation_lock = control::operation_lock(paths).await?;
    if state.registered {
        for command in &plan.remove {
            execute(runner, command).await?;
        }
    }
    remove_enable_link(plan)?;
    cleanup::remove_file(&plan.definition)?;
    for command in &plan.reload {
        execute(runner, command).await?;
    }
    wait_for_manager(&plan.manager, runner, true).await?;

    if purge {
        delete_credentials
            .await
            .context(Message::ServiceUninstalledButCredentialDeletionFailedRetryUninstall)?;
    }
    cleanup::remove_file(&plan.executable)
        .context(Message::ServiceUninstalledButInstalledExecutableCouldNotBe)?;
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
    #[cfg(any(target_os = "linux", test))]
    match &plan.manager {
        Manager::Systemd { system } => {
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
                        target == plan.definition
                            || target == std::path::Path::new("..").join(name),
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
        #[cfg(test)]
        _ => {}
    }
    #[cfg(not(any(target_os = "linux", test)))]
    let _ = plan;
    Ok(())
}

async fn execute(runner: &impl Runner, command: &Invocation) -> Result<()> {
    let output = runner.output(command).await?;
    if !command.optional {
        output.check(command)?;
    }
    Ok(())
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
    use crate::infra::command::tests::{FakeRunner, RUNNING, STOPPED};
    use crate::system::shared::services::artifacts as definition;
    use std::{cell::Cell, fs, path::Path};

    fn fixture(root: &Path, kind: usize) -> Result<(Paths, Plan)> {
        let base = root.join("cxcs-tools/wlanlogin");
        let mut paths = Paths::test(base.join("state"));
        paths.runtime = base.join("run");
        paths.secrets = base.join("secrets");
        let executable = base.join("bin/daemon");
        let definition = root.join("shared-services/service");
        let log = paths.config.join("daemon.log");
        let plan = match kind {
            0 => definition::linux_plan(executable, definition)?,
            1 => definition::linux_system_plan(executable, definition)?,
            2 => definition::macos_plan(executable, definition, log, 501)?,
            3 => definition::macos_system_plan(executable, definition, log)?,
            4 => definition::windows_plan(executable, definition, "S-1-5-21-123")?,
            _ => definition::windows_system_plan(executable, definition)?,
        };
        paths.scope = plan.scope;
        for path in [
            &paths.config,
            &paths.runtime,
            &paths.secrets,
            plan.executable.parent().context("binary parent")?,
            plan.definition.parent().context("service parent")?,
        ] {
            fs::create_dir_all(path)?;
        }
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "installed copy, never executed")?;
        for name in ["preferences.json", "control.json", "daemon.log"] {
            fs::write(paths.config.join(name), "test data")?;
        }
        for name in ["daemon-endpoint.json", "daemon.sock"] {
            fs::write(paths.runtime.join(name), "test data")?;
        }
        fs::write(paths.secrets.join("login.cred"), "encrypted test secret")?;
        fs::write(root.join("shared-services/another-service"), "preserve")?;
        fs::write(root.join("original-cli"), "preserve")?;
        Ok((paths, plan))
    }

    fn removal_runner(kind: usize) -> FakeRunner {
        FakeRunner::new(match kind {
            0 | 1 => vec![
                (true, STOPPED),
                (true, ""),
                (true, ""),
                (true, "LoadState=not-found\n"),
            ],
            2 | 3 => vec![
                (true, "job = {\nstate = not running\n}\n"),
                (true, ""),
                (false, "missing"),
                (true, "domain = {}"),
            ],
            4 => vec![
                (true, "Installed=1\nState=3\n"),
                (true, ""),
                (true, ""),
                (true, "Installed=0\n"),
            ],
            _ => vec![
                (true, "Installed=1\nState=1\n"),
                (true, ""),
                (true, ""),
                (true, "Installed=0\n"),
            ],
        })
    }

    #[tokio::test]
    async fn purge_cleans_each_platform_and_scope_without_removing_shared_files() -> Result<()> {
        for kind in 0..6 {
            let dir = tempfile::tempdir()?;
            let (paths, plan) = fixture(dir.path(), kind)?;
            let sibling = dir.path().join("cxcs-tools/another-tool");
            fs::create_dir(&sibling)?;
            fs::write(sibling.join("settings.json"), "preserve")?;
            let runner = removal_runner(kind);
            let deleted = Cell::new(false);
            uninstall_plan(&paths, &plan, true, &runner, async {
                deleted.set(true);
                cleanup::remove_file(&paths.secrets.join("login.cred"))
            })
            .await?;
            assert!(deleted.get());
            assert!(!plan.definition.exists() && !plan.executable.exists());
            assert!(!paths.config.exists() && !paths.runtime.exists() && !paths.secrets.exists());
            assert!(!dir.path().join("cxcs-tools/wlanlogin").exists());
            assert_eq!(
                fs::read_to_string(sibling.join("settings.json"))?,
                "preserve"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("shared-services/another-service"))?,
                "preserve"
            );
            assert!(dir.path().join("original-cli").exists());
            assert!(
                runner.outputs.borrow().is_empty(),
                "all service checks must run"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_uninstall_preserves_credentials_logs_and_preferences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        uninstall_plan(&paths, &plan, false, &removal_runner(0), async {
            panic!("ordinary uninstall must not delete credentials")
        })
        .await?;
        assert!(!plan.definition.exists() && !plan.executable.exists());
        assert!(paths.secrets.join("login.cred").exists());
        for name in ["preferences.json", "control.json", "daemon.log"] {
            assert!(paths.config.join(name).exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn stale_registration_is_removed_even_without_a_definition_file() -> Result<()> {
        for kind in 0..6 {
            let dir = tempfile::tempdir()?;
            let (paths, plan) = fixture(dir.path(), kind)?;
            fs::remove_file(&plan.definition)?;
            uninstall_plan(&paths, &plan, false, &removal_runner(kind), async {
                Ok(())
            })
            .await?;
            assert!(!plan.executable.exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_unregister_preserves_files_and_never_deletes_credentials() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        let runner = FakeRunner::new([(true, STOPPED), (false, "permission denied")]);
        assert!(
            uninstall_plan(&paths, &plan, true, &runner, async {
                panic!("credentials cannot be deleted after failed unregister")
            })
            .await
            .is_err()
        );
        assert!(plan.definition.exists() && plan.executable.exists());
        assert!(paths.secrets.join("login.cred").exists());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn purge_waits_for_running_service_before_unregistering_or_deleting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        let runner = FakeRunner::new([
            (true, RUNNING),
            (true, ""), // stop
            (true, RUNNING),
            (true, STOPPED),
            (true, ""), // disable
            (true, ""), // reload
            (true, "LoadState=not-found\n"),
        ]);
        uninstall_plan(&paths, &plan, true, &runner, async {
            assert_eq!(runner.calls.borrow().len(), 7);
            assert!(!plan.definition.exists());
            assert!(plan.executable.exists());
            cleanup::remove_file(&paths.secrets.join("login.cred"))
        })
        .await?;
        let calls = runner.calls.borrow();
        assert_eq!(calls[1].args[1], "stop");
        assert_eq!(calls[4].args[1], "disable");
        assert!(!paths.config.exists());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn failed_stop_never_deletes_service_files_or_credentials() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        // An optional stop command can fail, but its outcome must be verified.
        let runner = FakeRunner::new(
            [(true, RUNNING), (false, "stop denied")]
                .into_iter()
                .chain(std::iter::repeat_n((true, RUNNING), 130)),
        );
        let result = uninstall_plan(&paths, &plan, true, &runner, async {
            panic!("a running service must retain its credentials")
        })
        .await;
        assert!(
            format!("{:#}", result.err().context("expected stop timeout")?)
                .contains("Timed out waiting for service stop")
        );
        assert!(plan.definition.exists() && plan.executable.exists());
        assert!(paths.secrets.join("login.cred").exists());
        assert!(paths.config.join("daemon.log").exists());
        assert!(
            runner
                .calls
                .borrow()
                .iter()
                .all(|call| { !call.args.iter().any(|argument| argument == "disable") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn credential_failure_retains_data_and_can_be_retried_after_service_removal() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        let result = uninstall_plan(&paths, &plan, true, &removal_runner(0), async {
            crate::infra::error::bail!("keyring unavailable")
        })
        .await;
        assert!(
            format!("{:#}", result.err().context("expected credential failure")?)
                .contains("keyring unavailable")
        );
        assert!(!plan.definition.exists());
        assert!(plan.executable.exists());
        assert!(paths.config.join("daemon.log").exists());
        let runner = FakeRunner::new([
            (true, "LoadState=not-found\n"),
            (true, ""),
            (true, "LoadState=not-found\n"),
        ]);
        uninstall_plan(&paths, &plan, true, &runner, async {
            cleanup::remove_file(&paths.secrets.join("login.cred"))
        })
        .await?;
        assert!(!plan.executable.exists());
        assert!(!dir.path().join("cxcs-tools/wlanlogin").exists());
        Ok(())
    }

    #[cfg(unix)]
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

    #[tokio::test]
    async fn purge_is_repeatable_and_preserves_unknown_files_in_app_directories() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        fs::write(paths.config.join("personal-note"), "preserve")?;
        uninstall_plan(&paths, &plan, true, &removal_runner(0), async {
            cleanup::remove_file(&paths.secrets.join("login.cred"))
        })
        .await?;
        let runner = FakeRunner::new([
            (true, "LoadState=not-found\n"),
            (true, ""),
            (true, "LoadState=not-found\n"),
        ]);
        uninstall_plan(&paths, &plan, true, &runner, async { Ok(()) }).await?;
        assert_eq!(
            fs::read_to_string(paths.config.join("personal-note"))?,
            "preserve"
        );
        assert!(!paths.config.join("daemon.log").exists());
        assert!(!paths.runtime.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn purge_rejects_redirected_data_roots_before_touching_services() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut paths, plan) = fixture(dir.path(), 0)?;
        let outside = dir.path().join("unrelated");
        fs::create_dir(&outside)?;
        fs::write(outside.join("daemon.log"), "preserve")?;
        let link = dir.path().join("redirect");
        std::os::unix::fs::symlink(&outside, &link)?;
        paths.config = link;
        let runner = FakeRunner::default();
        assert!(
            uninstall_plan(&paths, &plan, true, &runner, async { Ok(()) })
                .await
                .is_err()
        );
        assert!(runner.calls.borrow().is_empty());
        assert_eq!(fs::read_to_string(outside.join("daemon.log"))?, "preserve");
        assert!(plan.definition.exists());
        Ok(())
    }
}
