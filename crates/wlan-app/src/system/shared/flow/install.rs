//! Own the sequence and recovery barriers for installing a managed service.
use super::hooks::{Hooks, step};
use crate::infra::{
    command::{Runner, SystemRunner},
    control,
    error::{Context, Error, Message, Result, ensure},
    paths::Paths,
};
use crate::system::{
    define::services::Plan,
    shared::services::{
        artifacts::{assert_owned, plan},
        payload::Backup,
        steps::execute_optional,
        transaction::Rollback,
    },
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Validate,
    Lock,
    Stage,
    Snapshot,
    StopPrevious,
    Publish,
    Register,
    Verify,
    Commit,
    StopFailed,
    RemoveFailed,
    RestorePayload,
    RestoreRegistration,
    RetainRecovery,
}

pub(crate) struct InstallContext<'a, R> {
    pub paths: &'a Paths,
    pub plan: &'a Plan,
    pub previous: &'a Plan,
    pub source: &'a Path,
    pub runner: &'a R,
}

pub(crate) trait Payload: Sized {
    fn publish(&mut self, plan: &Plan) -> Result<()>;
    fn restore(&mut self, plan: &Plan) -> Result<()>;
    fn retain(self) -> Result<Vec<PathBuf>>;
    fn commit(self) {}
}

pub(crate) trait Backend<R: Runner>: for<'a> Hooks<Step, InstallContext<'a, R>> {
    type Staged: Payload;
    type Registration;

    fn target_plan(&self, paths: &Paths) -> Result<Plan> {
        plan(paths)
    }
    fn previous_plan(&self, _paths: &Paths) -> Result<Option<Plan>> {
        Ok(None)
    }
    fn source(&self) -> Result<PathBuf> {
        Ok(std::env::current_exe()?)
    }
    async fn stage(&self, context: &InstallContext<'_, R>) -> Result<Self::Staged>;
    async fn snapshot(&self, context: &InstallContext<'_, R>) -> Result<Self::Registration>;
    async fn stop(&self, context: &InstallContext<'_, R>, plan: &Plan) -> Result<()> {
        for command in &plan.stop {
            execute_optional(context.runner, command).await?;
        }
        control::stop(context.paths).await;
        control::wait_stopped(context.paths).await
    }
    async fn remove_failed(&self, context: &InstallContext<'_, R>) -> Result<()> {
        for command in &context.plan.remove {
            execute_optional(context.runner, command).await?;
        }
        Ok(())
    }
    async fn verify(&self, _context: &InstallContext<'_, R>) -> Result<bool> {
        Ok(true)
    }
    async fn restore_registration(
        &self,
        context: &InstallContext<'_, R>,
        registration: &Self::Registration,
    ) -> Result<()>;
    fn retain_registration(
        &self,
        _context: &InstallContext<'_, R>,
        _registration: &Self::Registration,
    ) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    fn restored_error(&self) -> Message {
        Message::InstallationFailedInstallationFilesRestored
    }
    fn incomplete_error(&self, failures: String, backup: String) -> Message {
        Message::InstallationFailedAndRollbackWasIncomplete {
            value0: format!("{failures}; recovery files: {backup}"),
        }
    }
    fn report_installed(&self, paths: &Paths, _ready: bool) {
        tracing::info!(
            "{}",
            Message::ServiceInstalledAndStartedForScope {
                value0: paths.scope.label().to_string(),
                value1: if paths.scope == crate::infra::scope::Scope::System {
                    "autostart enabled at boot"
                } else {
                    "autostart enabled at user login"
                }
                .to_string(),
            }
        );
    }
}

pub(crate) async fn run(paths: &Paths) -> Result<()> {
    let backend = crate::system::native::flow::install::NativeHooks;
    validate_scope(paths)?;
    let plan = Backend::<SystemRunner>::target_plan(&backend, paths)?;
    let previous = Backend::<SystemRunner>::previous_plan(&backend, paths)?;
    let source = Backend::<SystemRunner>::source(&backend)?;
    run_plan(
        &InstallContext {
            paths,
            plan: &plan,
            previous: previous.as_ref().unwrap_or(&plan),
            source: &source,
            runner: &SystemRunner,
        },
        &backend,
    )
    .await
}

pub(crate) async fn run_plan<R: Runner>(
    context: &InstallContext<'_, R>,
    backend: &impl Backend<R>,
) -> Result<()> {
    let InstallContext {
        paths,
        plan,
        previous,
        runner,
        ..
    } = *context;
    step(backend, Step::Validate, context, async {
        ensure!(
            paths.scope == plan.scope && paths.scope == previous.scope,
            Message::ServicePlanDoesNotMatchTheSelected
        );
        Ok(())
    })
    .await?;
    let _lock = step(backend, Step::Lock, context, async {
        control::service_lock(paths)
    })
    .await?;
    let mut staged = step(backend, Step::Stage, context, async {
        assert_owned(&plan.definition)?;
        backend.stage(context).await
    })
    .await?;
    let snapshot = step(backend, Step::Snapshot, context, backend.snapshot(context)).await?;
    step(
        backend,
        Step::StopPrevious,
        context,
        backend.stop(context, previous),
    )
    .await?;

    let outcome = async {
        step(backend, Step::Publish, context, async {
            staged.publish(plan)
        })
        .await?;
        step(backend, Step::Register, context, async {
            for command in &plan.start {
                execute_optional(runner, command).await?;
            }
            Ok(())
        })
        .await?;
        let ready = step(backend, Step::Verify, context, backend.verify(context)).await?;
        backend.before(Step::Commit, context).await?;
        Ok::<_, Error>(ready)
    }
    .await;

    let ready = match outcome {
        Ok(ready) => ready,
        Err(error) => {
            // Never restore old artifacts beneath a process that failed to stop.
            let mut rollback = Rollback::default();
            rollback
                .step(step(
                    backend,
                    Step::StopFailed,
                    context,
                    backend.stop(context, plan),
                ))
                .await;
            rollback
                .step(step(
                    backend,
                    Step::RemoveFailed,
                    context,
                    backend.remove_failed(context),
                ))
                .await;
            rollback
                .step(step(backend, Step::RestorePayload, context, async {
                    staged.restore(plan)
                }))
                .await;
            rollback
                .step(step(
                    backend,
                    Step::RestoreRegistration,
                    context,
                    backend.restore_registration(context, &snapshot),
                ))
                .await;
            if rollback.safe() {
                return Err(error).context(backend.restored_error());
            }
            // Retention still runs if its before hook fails. Never discard recovery material.
            let before = backend.before(Step::RetainRecovery, context).await;
            let mut recovered = Vec::new();
            let payload = staged.retain().map(|files| recovered.extend(files));
            let registration = backend
                .retain_registration(context, &snapshot)
                .map(|path| recovered.extend(path));
            let retention = match (payload, registration) {
                (Err(primary), Err(secondary)) => {
                    rollback.record(Err(secondary));
                    Err(primary)
                }
                (first, second) => first.and(second),
            };
            let after = backend
                .after(
                    Step::RetainRecovery,
                    context,
                    retention.as_ref().map(|_| ()),
                )
                .await;
            rollback.record(retention);
            rollback.record(before);
            rollback.record(after);
            let recovered = recovered
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(error).context(backend.incomplete_error(rollback.detail(), recovered));
        }
    };
    staged.commit();
    backend.after(Step::Commit, context, Ok(())).await?;
    backend.report_installed(paths, ready);
    Ok(())
}

// Linux and Windows publish a standalone executable and a service definition.
#[allow(dead_code)]
pub(crate) struct FilePayload {
    executable: Option<tempfile::NamedTempFile>,
    definition: Option<tempfile::TempPath>,
    definition_backup: Backup,
    executable_backup: Backup,
}

#[allow(dead_code)]
impl FilePayload {
    pub(crate) fn stage(plan: &Plan, source: &Path, bytes: &[u8]) -> Result<Self> {
        let bin_dir = plan
            .executable
            .parent()
            .context(Message::InstallationPathHasNoParentDirectory)?;
        let definition_dir = plan
            .definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?;
        let source = source.canonicalize()?;
        let definition_backup = Backup::capture(&plan.definition)?;
        let executable_backup = Backup::capture(&plan.executable)?;
        let mut definition = tempfile::NamedTempFile::new_in(definition_dir)?;
        definition.write_all(bytes)?;
        definition.as_file().sync_all()?;
        let executable = if plan.executable.canonicalize().ok().as_ref() == Some(&source) {
            None
        } else {
            let executable = tempfile::NamedTempFile::new_in(bin_dir)?;
            fs::copy(&source, executable.path())?;
            Some(executable)
        };
        Ok(Self {
            executable,
            definition: Some(definition.into_temp_path()),
            definition_backup,
            executable_backup,
        })
    }

    pub(crate) fn executable(&self) -> Option<&Path> {
        self.executable.as_ref().map(|file| file.path())
    }
    pub(crate) fn definition(&self) -> Result<&Path> {
        self.definition
            .as_deref()
            .context(Message::ServiceDefinitionHasNoParentDirectory)
    }
}

impl Payload for FilePayload {
    fn publish(&mut self, plan: &Plan) -> Result<()> {
        if let Some(executable) = self.executable.take() {
            executable
                .persist(&plan.executable)
                .map_err(|error| error.error)?;
        }
        self.definition
            .take()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?
            .persist(&plan.definition)
            .map_err(|error| error.error)?;
        Ok(())
    }

    fn restore(&mut self, plan: &Plan) -> Result<()> {
        let executable = self.executable_backup.restore(&plan.executable);
        let definition = self.definition_backup.restore(&plan.definition);
        executable.and(definition)
    }

    fn retain(self) -> Result<Vec<PathBuf>> {
        let executable = self.executable_backup.retain();
        let definition = self.definition_backup.retain();
        Ok(executable?.into_iter().chain(definition?).collect())
    }
}
