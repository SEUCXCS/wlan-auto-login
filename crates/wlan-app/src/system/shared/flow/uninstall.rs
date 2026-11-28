//! Stop and unregister services before removing scoped application data.
use super::hooks::{Hooks, step};
use crate::infra::{
    cleanup,
    command::{Runner, SystemRunner},
    control,
    error::{Context, Message, Result, ensure},
    paths::Paths,
};
use crate::system::{
    define::services::{Manager, Plan, ServiceManager, ServiceState},
    shared::services::{
        artifacts::{assert_owned, plan},
        steps::execute_optional,
    },
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Validate,
    ServiceLock,
    Query,
    Stop,
    DataLocks,
    Unregister,
    VerifyUnregistered,
    PurgeCredentials,
    RemovePayload,
    PurgeFiles,
}

pub(crate) struct UninstallContext<'a> {
    pub paths: &'a Paths,
    pub plan: &'a Plan,
}

pub(crate) trait UninstallHooks: for<'a> Hooks<Step, UninstallContext<'a>> {
    fn must_stop(&self, _plan: &Plan, state: &ServiceState) -> bool {
        state.running || state.pending
    }
    async fn stop_registered(&self, plan: &Plan, runner: &impl Runner) -> Result<()> {
        for command in &plan.stop {
            execute_optional(runner, command).await?;
        }
        Ok(())
    }
    fn retain_locks(&self) -> bool {
        true
    }
    fn packaged_definition(&self, _plan: &Plan) -> bool {
        false
    }
    fn remove_payload(&self, plan: &Plan) -> Result<()> {
        cleanup::remove_file(&plan.executable)
            .context(Message::ServiceUninstalledButInstalledExecutableCouldNotBe)
    }
    fn runtime_files(&self, _paths: &Paths, _plan: &Plan) -> Vec<&'static str> {
        Vec::new()
    }
}

pub(crate) async fn run(paths: &Paths, purge: bool) -> Result<()> {
    run_plan(
        paths,
        &plan(paths)?,
        purge,
        &SystemRunner,
        crate::system::shared::credentials::delete(paths),
        &crate::system::native::flow::uninstall::NativeHooks,
    )
    .await?;
    if purge {
        tracing::info!(
            "{}",
            Message::ServiceAndApplicationDataRemovedForScope {
                value0: paths.scope.label().to_string()
            }
        );
        tracing::info!("{}", Message::SharedSystemLogsAndServiceAuditRecords);
    } else {
        tracing::info!("{}", Message::ServiceUninstalledSavedCredentialsRetained);
    }
    Ok(())
}

pub(crate) async fn run_plan(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
    hooks: &impl UninstallHooks,
) -> Result<()> {
    let context = UninstallContext { paths, plan };
    step(hooks, Step::Validate, &context, async {
        ensure!(
            context.plan.scope == context.paths.scope,
            Message::ServicePlanDoesNotMatchTheSelected
        );
        for directory in [&paths.config, &paths.runtime, &paths.secrets] {
            cleanup::validate_directory(directory)?;
        }
        assert_owned(&plan.definition)
    })
    .await?;
    let service = step(hooks, Step::ServiceLock, &context, async {
        control::service_lock(paths)
    })
    .await?;
    let state = step(hooks, Step::Query, &context, plan.manager.query(runner)).await?;
    step(hooks, Step::Stop, &context, async {
        if hooks.must_stop(plan, &state) {
            hooks.stop_registered(plan, runner).await?;
            wait_for_manager(&plan.manager, runner, false).await?;
        }
        control::stop(paths).await;
        control::wait_stopped(paths).await
    })
    .await?;
    let mut locks = Some(
        step(hooks, Step::DataLocks, &context, async {
            Ok((
                service,
                control::singleton(paths)?,
                control::operation_lock(paths).await?,
            ))
        })
        .await?,
    );

    step(hooks, Step::Unregister, &context, async {
        if state.registered {
            for command in &plan.remove {
                execute_optional(runner, command).await?;
            }
        }
        Ok(())
    })
    .await?;
    step(hooks, Step::VerifyUnregistered, &context, async {
        // Signed definitions stay inside their bundle until credential cleanup succeeds.
        if !hooks.packaged_definition(plan) {
            cleanup::remove_file(&plan.definition)?;
        }
        for command in &plan.reload {
            execute_optional(runner, command).await?;
        }
        wait_for_manager(&plan.manager, runner, true).await
    })
    .await?;
    if purge {
        step(hooks, Step::PurgeCredentials, &context, async {
            delete_credentials
                .await
                .context(Message::ServiceUninstalledButCredentialDeletionFailedRetryUninstall)
        })
        .await?;
    }
    step(hooks, Step::RemovePayload, &context, async {
        hooks.remove_payload(plan)
    })
    .await?;
    if purge {
        step(hooks, Step::PurgeFiles, &context, async {
            for name in [
                "preferences.json",
                "control.json",
                "daemon.log",
                crate::infra::config::CONFIG_FILE,
            ] {
                cleanup::remove_file(&paths.config.join(name))?;
            }
            for name in ["daemon-endpoint.json", "daemon.sock"] {
                cleanup::remove_file(&paths.runtime.join(name))?;
            }
            for name in hooks.runtime_files(paths, plan) {
                cleanup::remove_file(&paths.runtime.join(name))?;
            }
            if !hooks.retain_locks() {
                drop(locks.take());
                for name in ["operation.lock", "daemon.lock"] {
                    cleanup::remove_file(&paths.runtime.join(name))?;
                }
                cleanup::remove_file(&paths.config.join("service.lock"))?;
            }
            for directory in [&paths.runtime, &paths.secrets, &paths.config] {
                cleanup::prune_empty_directories(directory)?;
            }
            cleanup::prune_empty_directories(
                plan.executable
                    .parent()
                    .context(Message::InstallationPathHasNoParentDirectory)?,
            )
        })
        .await?;
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
pub(crate) async fn uninstall_plan(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
) -> Result<()> {
    run_plan(
        paths,
        plan,
        purge,
        runner,
        delete_credentials,
        &super::tests::FixtureHooks,
    )
    .await
}
