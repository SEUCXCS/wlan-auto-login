//! Install a verified app as one transaction, including Keychain identity checks.
use crate::infra::{
    cleanup,
    command::{Invocation, Runner, SystemRunner},
    control,
    error::{Context, Error, Message, Result},
    paths::{self, Paths},
    scope::Scope,
};
use crate::system::macos::services::payload::{self, Payload};
use crate::system::{
    define::{
        permission::PermissionStatus,
        services::{Action, Plan, ServiceManager},
    },
    macos::{credentials::access, runtime::host, services::bundle},
};
use std::{fs, future::Future, path::Path};

use crate::system::define::services::{Manager, ServiceState};
use crate::system::shared::flow::{
    hooks::Hooks,
    install::{Backend, InstallContext, Payload as InstallPayload, Step},
};
use std::path::PathBuf;

pub(crate) struct NativeHooks;
impl<R: Runner> Hooks<Step, InstallContext<'_, R>> for NativeHooks {
    async fn before(&self, step: Step, context: &InstallContext<'_, R>) -> Result<()> {
        match step {
            Step::Publish => {
                let destination = paths::macos_bundle(context.paths.scope)?;
                let app = if destination.exists() {
                    &destination
                } else {
                    context.source
                };
                host::stop_helpers(context.paths.scope, app).await?;
                unregister(context.previous).await?;
            }
            Step::Register => {
                prepare_credentials(context.paths.scope, &context.plan.executable).await?;
                if let Err(error) = host::prepare(access::interaction_allowed()).await {
                    if context.paths.scope == Scope::System && !access::interaction_allowed() {
                        tracing::warn!(%error, "System service will wait for an authorized GUI-session Wi-Fi host");
                    } else {
                        return Err(error);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn after(
        &self,
        step: Step,
        context: &InstallContext<'_, R>,
        result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        if result.is_err() {
            return Ok(());
        }
        match step {
            Step::Validate if context.paths.scope == Scope::System => prepare_system_directories()?,
            Step::Publish if context.paths.scope == Scope::System => {
                use std::os::unix::fs::PermissionsExt;
                let destination = paths::macos_bundle(context.paths.scope)?;
                fs::set_permissions(
                    destination
                        .parent()
                        .context(Message::InstallationPathHasNoParentDirectory)?,
                    fs::Permissions::from_mode(0o755),
                )?;
            }
            Step::RestorePayload => {
                let original = std::env::current_exe()?;
                let executable = if context.plan.executable.is_file() {
                    &context.plan.executable
                } else {
                    &original
                };
                prepare_credentials(context.paths.scope, executable).await?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl<R: Runner> Backend<R> for NativeHooks {
    type Staged = Payload;
    type Registration = ServiceState;

    fn target_plan(&self, paths: &Paths) -> Result<Plan> {
        crate::system::macos::services::definition::native_plan(paths)
    }
    fn previous_plan(&self, paths: &Paths) -> Result<Option<Plan>> {
        crate::system::macos::services::definition::plan(paths).map(Some)
    }
    fn source(&self) -> Result<PathBuf> {
        bundle::source()
    }

    async fn stage(&self, context: &InstallContext<'_, R>) -> Result<Payload> {
        cleanup::validate_directory(
            context
                .plan
                .definition
                .parent()
                .context(Message::ServiceDefinitionHasNoParentDirectory)?,
        )?;
        let source = context.source;
        payload::validate(source).await?;
        for system in [false, true] {
            let definition = source.join(crate::infra::identity::macos_service_path(system));
            let expected = if system == (context.paths.scope == Scope::System) {
                context.plan.content.clone()
            } else {
                crate::infra::identity::macos_service_plist(system)
            };
            crate::infra::error::ensure!(
                fs::read_to_string(&definition).ok().as_deref() == Some(expected.as_str()),
                Message::MacosServiceDefinitionMissing
            );
        }
        let destination = paths::macos_bundle(context.paths.scope)?;
        let staged = Payload::stage(source, &destination)?;
        payload::validate(&staged.staged()).await?;
        if destination.exists() {
            payload::validate(&destination).await?;
        }
        Ok(staged)
    }

    async fn snapshot(&self, context: &InstallContext<'_, R>) -> Result<ServiceState> {
        context.previous.manager.query(context.runner).await
    }
    async fn stop(&self, context: &InstallContext<'_, R>, plan: &Plan) -> Result<()> {
        stop(context.paths, plan).await
    }
    async fn remove_failed(&self, context: &InstallContext<'_, R>) -> Result<()> {
        host::stop_helpers(
            context.paths.scope,
            &paths::macos_bundle(context.paths.scope)?,
        )
        .await?;
        unregister(context.plan).await
    }
    async fn verify(&self, context: &InstallContext<'_, R>) -> Result<bool> {
        crate::system::macos::services::readiness::wait_installed(context.paths, context.plan).await
    }
    async fn restore_registration(
        &self,
        context: &InstallContext<'_, R>,
        state: &ServiceState,
    ) -> Result<()> {
        if state.registered {
            for command in &context.previous.start {
                crate::system::macos::services::app_service::output(command, context.runner)
                    .await?;
            }
            if !state.running
                && !state.pending
                && matches!(context.previous.manager, Manager::MacosAppService { .. })
            {
                stop(context.paths, context.previous).await?;
            }
        }
        Ok(())
    }
    fn restored_error(&self) -> Message {
        Message::BundleInstallationRestored
    }
    fn incomplete_error(&self, failures: String, backup: String) -> Message {
        Message::BundleRollbackIncomplete { failures, backup }
    }
    fn report_installed(&self, paths: &Paths, ready: bool) {
        if ready {
            tracing::info!(
                scope = paths.scope.label(),
                "Signed application installed and managed daemon ready"
            );
        } else {
            tracing::warn!(
                scope = paths.scope.label(),
                "{}",
                Message::MacosBackgroundApprovalRequired
            );
        }
    }
}

impl InstallPayload for Payload {
    fn publish(&mut self, _plan: &Plan) -> Result<()> {
        Payload::publish(self)
    }
    fn restore(&mut self, _plan: &Plan) -> Result<()> {
        Payload::restore(self)
    }
    fn retain(mut self) -> Result<Vec<PathBuf>> {
        Ok(vec![Payload::retain(&mut self)])
    }
    fn commit(self) {
        Payload::commit(self);
    }
}

async fn unregister(plan: &Plan) -> Result<()> {
    if matches!(
        plan.manager,
        crate::system::define::services::Manager::MacosAppService { .. }
    ) && plan.definition.try_exists()?
    {
        for command in &plan.remove {
            crate::system::macos::services::app_service::output(command, &SystemRunner).await?;
        }
    }
    Ok(())
}

async fn authorize(scope: Scope, executable: &Path) -> Result<()> {
    let executable = executable.to_owned();
    tokio::time::timeout(
        std::time::Duration::from_secs(90),
        tokio::task::spawn_blocking(move || access::authorize_binary(scope, &executable)),
    )
    .await
    .context(Message::KeychainAuthorizationTimedOut)??
}
async fn prepare_credentials(scope: Scope, executable: &Path) -> Result<()> {
    ensure_credential_access(
        || credential_status(scope, executable, &SystemRunner),
        || async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(90),
                tokio::task::spawn_blocking(move || access::unlock_if_needed(scope)),
            )
            .await
            .context(Message::KeychainAuthorizationTimedOut)?
            .context(Message::KeychainUnlockFailed)?
        },
        || authorize(scope, executable),
    )
    .await
}

async fn ensure_credential_access<P, U, A>(
    mut probe: impl FnMut() -> P,
    unlock: impl FnOnce() -> U,
    authorize: impl FnOnce() -> A,
) -> Result<()>
where
    P: Future<Output = Result<PermissionStatus>>,
    U: Future<Output = Result<bool>>,
    A: Future<Output = Result<()>>,
{
    let mut status = probe().await?;
    if status == PermissionStatus::Granted {
        return Ok(());
    }
    require_authorizable(&status)?;
    if unlock().await? {
        status = probe().await?;
        if status == PermissionStatus::Granted {
            return Ok(());
        }
        require_authorizable(&status)?;
    }
    authorize().await?;
    let status = probe().await?;
    crate::infra::error::ensure!(
        status == PermissionStatus::Granted,
        Message::InstalledCredentialAccessUnavailable {
            state: format!("{status:?}")
        }
    );
    Ok(())
}

fn require_authorizable(status: &PermissionStatus) -> Result<()> {
    crate::infra::error::ensure!(
        matches!(
            status,
            PermissionStatus::NeedsInteraction | PermissionStatus::Denied
        ),
        Message::InstalledCredentialAccessUnavailable {
            state: format!("{status:?}")
        }
    );
    Ok(())
}

async fn credential_status(
    scope: Scope,
    executable: &Path,
    runner: &impl Runner,
) -> Result<PermissionStatus> {
    let flag = if scope == Scope::System {
        "--system"
    } else {
        "--user"
    };
    let command = Invocation::new(
        executable,
        [flag, "permission", "check", "macos.keychain", "--json"],
    );
    let output = runner
        .output(&command)
        .await
        .context(Message::InstalledCredentialCheckFailed)?;
    output
        .check(&command)
        .context(Message::InstalledCredentialCheckFailed)?;
    serde_json::from_str::<crate::system::define::permission::PermissionReport>(&output.stdout)
        .context(Message::InstalledCredentialCheckProtocol)?
        .into_status(scope.label(), "macos.keychain")
}

async fn stop(paths: &Paths, plan: &Plan) -> Result<()> {
    let state = plan.manager.query(&SystemRunner).await?;
    plan.manager
        .manage(plan, Action::Stop, state, &SystemRunner)
        .await?;
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    control::stop_managed(paths).await
}

fn prepare_system_directories() -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let application = paths::app_dir("/Library/Application Support");
    let shared = application
        .parent()
        .context(Message::InstallationPathHasNoParentDirectory)?;
    cleanup::validate_directory(&application)?;
    if !shared.try_exists()? {
        fs::create_dir(shared)?;
        fs::set_permissions(shared, fs::Permissions::from_mode(0o755))?;
    }
    let metadata = fs::metadata(shared)?;
    crate::infra::error::ensure!(
        metadata.uid() == 0 && metadata.mode() & 0o022 == 0 && metadata.mode() & 0o005 == 0o005,
        Message::SystemApplicationDirectoryPermissions {
            path: shared.display().to_string()
        }
    );
    fs::create_dir_all(&application)?;
    let metadata = fs::metadata(&application)?;
    crate::infra::error::ensure!(
        metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        Message::SystemApplicationDirectoryPermissions {
            path: application.display().to_string()
        }
    );
    fs::set_permissions(application, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::FakeRunner;
    use std::cell::RefCell;

    #[tokio::test]
    async fn repeated_install_with_read_access_never_unlocks_or_rewrites_acl() -> Result<()> {
        for _ in 0..3 {
            ensure_credential_access(
                || async { Ok(PermissionStatus::Granted) },
                || async { panic!("authorized installs must not request unlock") },
                || async { panic!("authorized installs must not rewrite access") },
            )
            .await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn locked_keychain_is_rechecked_before_any_acl_change() -> Result<()> {
        let statuses = RefCell::new(
            [
                PermissionStatus::NeedsInteraction,
                PermissionStatus::Granted,
            ]
            .into_iter(),
        );
        ensure_credential_access(
            || async {
                statuses
                    .borrow_mut()
                    .next()
                    .context(Message::UnexpectedServiceCommand)
            },
            || async { Ok(true) },
            || async { panic!("unlocking was sufficient; owner and ACL must be retained") },
        )
        .await?;
        assert!(statuses.borrow_mut().next().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn missing_or_unavailable_credentials_and_failed_probes_do_not_modify_access() {
        for status in [
            PermissionStatus::Missing,
            PermissionStatus::Unavailable,
            PermissionStatus::Unknown,
        ] {
            assert!(
                ensure_credential_access(
                    || {
                        let status = status.clone();
                        async move { Ok(status) }
                    },
                    || async { panic!("unavailable items must be retained") },
                    || async { panic!("unavailable items must not be authorized") },
                )
                .await
                .is_err()
            );
        }
        assert!(
            ensure_credential_access(
                || async { crate::infra::error::bail!(Message::InstalledCredentialCheckFailed) },
                || async { panic!("failed process probes must not request unlock") },
                || async { panic!("failed process probes must not modify access") },
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn changed_identity_requires_successful_installed_recheck() {
        for final_status in [PermissionStatus::Granted, PermissionStatus::Denied] {
            let statuses = RefCell::new(
                [PermissionStatus::NeedsInteraction, final_status.clone()].into_iter(),
            );
            let calls = RefCell::new(Vec::new());
            let result = ensure_credential_access(
                || async {
                    calls.borrow_mut().push("probe");
                    statuses
                        .borrow_mut()
                        .next()
                        .context(Message::UnexpectedServiceCommand)
                },
                || async {
                    calls.borrow_mut().push("unlock");
                    Ok(false)
                },
                || async {
                    calls.borrow_mut().push("authorize");
                    Ok(())
                },
            )
            .await;
            assert_eq!(result.is_ok(), final_status == PermissionStatus::Granted);
            assert_eq!(*calls.borrow(), ["probe", "unlock", "authorize", "probe"]);
        }
    }

    #[tokio::test]
    async fn installed_probe_rejects_failed_malformed_or_incompatible_results() -> Result<()> {
        for (success, stdout) in [
            (false, ""),
            (true, ""),
            (true, r#"{"version":2,"status":"Granted"}"#),
        ] {
            let runner = FakeRunner::new([(success, stdout)]);
            assert!(
                credential_status(Scope::User, Path::new("/isolated/app"), &runner,)
                    .await
                    .is_err()
            );
        }
        let runner = FakeRunner::new([(
            true,
            r#"{"version":1,"scope":"system","permissions":[{"id":"macos.keychain","status":"Granted","error":null}]}"#,
        )]);
        assert_eq!(
            credential_status(Scope::System, Path::new("/isolated/app"), &runner,).await?,
            PermissionStatus::Granted
        );
        assert_eq!(
            runner.calls.borrow()[0].args,
            [
                "--system",
                "permission",
                "check",
                "macos.keychain",
                "--json"
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_unlock_or_authorization_returns_to_the_install_transaction() {
        let result = ensure_credential_access(
            || async { Ok(PermissionStatus::NeedsInteraction) },
            || async { crate::infra::error::bail!(Message::KeychainUnlockFailed) },
            || async { panic!("failed unlock must not change ACL") },
        )
        .await;
        assert!(result.is_err());
        let calls = RefCell::new(0);
        let result = ensure_credential_access(
            || async {
                *calls.borrow_mut() += 1;
                Ok(PermissionStatus::NeedsInteraction)
            },
            || async { Ok(false) },
            || async {
                crate::infra::error::bail!(Message::KeychainAuthorizeFailed { status: -128 })
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(*calls.borrow(), 1);
    }
}
