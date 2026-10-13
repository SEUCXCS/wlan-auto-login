//! Install a verified app as one transaction, including Keychain identity checks.
use crate::infra::{
    cleanup,
    command::{Invocation, Runner, SystemRunner},
    control,
    error::{Context, Message, Result},
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

async fn install(paths: &Paths, previous_plan: Plan) -> Result<()> {
    // Validate the selected scope before preparing or changing native artifacts.
    crate::infra::error::ensure!(
        paths.scope == previous_plan.scope,
        crate::infra::error::Message::ServicePlanDoesNotMatchTheSelected
    );

    // Acquire the service lock and validate ownership of the signed definitions.
    let plan = crate::system::macos::services::definition::native_plan(paths)?;
    if paths.scope == Scope::System {
        prepare_system_directories()?;
    }
    let _lock = control::service_lock(paths)?;
    crate::system::shared::services::artifacts::assert_owned(&plan.definition)?;
    cleanup::validate_directory(
        plan.definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;

    // Verify the complete source bundle and both immutable signed job definitions.
    let source = bundle::source()?;
    payload::validate(&source).await?;
    for system in [false, true] {
        let definition = source.join(crate::infra::identity::macos_service_path(system));
        let expected = if system == (paths.scope == Scope::System) {
            plan.content.clone()
        } else {
            crate::infra::identity::macos_service_plist(system)
        };
        crate::infra::error::ensure!(
            fs::read_to_string(&definition).ok().as_deref() == Some(expected.as_str()),
            Message::MacosServiceDefinitionMissing
        );
    }

    // Stage the full payload and validate both staged and existing identities.
    let destination = paths::macos_bundle(paths.scope)?;
    let mut transaction = Payload::stage(&source, &destination)?;
    payload::validate(&transaction.staged()).await?;
    if destination.exists() {
        payload::validate(&destination).await?;
    }

    // Capture liveness, then stop the old job before replacing signed code.
    let state = previous_plan.manager.query(&SystemRunner).await?;
    stop(paths, &previous_plan).await?;

    let outcome = async {
        // Stop authenticated transient helpers before unregistration/replacement.
        host::stop_helpers(
            paths.scope,
            if destination.exists() {
                &destination
            } else {
                &source
            },
        )
        .await?;

        // Unregister using the old signed bundle before replacing it. A raw
        // bootout alone preserves native registration for future logins.
        unregister(&previous_plan).await?;
        transaction.publish()?;

        // Application root is public code, while state/secrets stay private.
        if paths.scope == Scope::System {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                destination.parent().context(Message::InstallationPathHasNoParentDirectory)?,
                fs::Permissions::from_mode(0o755),
            )?;
        }

        // Check access from the installed identity; repair only a known authorization need.
        prepare_credentials(paths.scope, &plan.executable).await?;

        // Authorize the newly installed identity, which can differ after ad-hoc signing.
        if let Err(error) = host::prepare(access::interaction_allowed()).await {
            if paths.scope == Scope::System && !access::interaction_allowed() {
                tracing::warn!(%error, "System service will wait for an authorized GUI-session Wi-Fi host");
            } else { return Err(error); }
        }

        // Packaged definitions were signed by xtask. Installation never rewrites them.
        for command in &plan.start {
            crate::system::macos::services::app_service::output(command, &SystemRunner).await?;
        }

        // Background approval is distinct from managed-daemon liveness.
        let registered = plan.manager.query(&SystemRunner).await?;
        if registered.registered && registered.enabled == Some(false) {
            return Ok::<_, crate::infra::error::Error>(false);
        }
        crate::system::macos::services::readiness::wait_ready(paths, &plan).await?;
        Ok(true)
    }
    .await;

    // Reuse the shared stop barrier, retaining signed recovery data on failure.
    if let Err(error) = outcome {
        // Never restore files underneath a process which failed to stop.
        let mut rollback = crate::system::shared::services::transaction::Rollback::default();
        rollback.step(stop(paths, &plan)).await;
        rollback
            .step(host::stop_helpers(paths.scope, &destination))
            .await;
        rollback.step(unregister(&plan)).await;
        let mut failures = if rollback.safe() {
            Vec::new()
        } else {
            vec![rollback.detail()]
        };
        if failures.is_empty() {
            if let Err(e) = transaction.restore() {
                failures.push(e.to_string());
            }
            if failures.is_empty() {
                let original = std::env::current_exe()?;
                let executable = if transaction.existed() {
                    &plan.executable
                } else {
                    &original
                };
                if let Err(e) = prepare_credentials(paths.scope, executable).await {
                    failures.push(e.to_string());
                }
            }
            if failures.is_empty() && state.registered {
                for command in &previous_plan.start {
                    if let Err(e) =
                        crate::system::macos::services::app_service::output(command, &SystemRunner)
                            .await
                    {
                        failures.push(e.to_string());
                    }
                }
                // Native registration survives a session-only stop. Restore
                // that registration without resuming a previously stopped job.
                if failures.is_empty()
                    && !state.running
                    && !state.pending
                    && matches!(
                        previous_plan.manager,
                        crate::system::define::services::Manager::MacosAppService { .. }
                    )
                    && let Err(e) = stop(paths, &previous_plan).await
                {
                    failures.push(e.to_string());
                }
            }
        }
        if !failures.is_empty() {
            let backup = transaction.retain();
            return Err(error).context(Message::BundleRollbackIncomplete {
                failures: failures.join("; "),
                backup: backup.display().to_string(),
            });
        }
        return Err(error).context(Message::BundleInstallationRestored);
    }

    // Commit only a successful native registration or a known pending-approval state.
    let ready = outcome?;
    transaction.commit();
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
    Ok(())
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

/// Prepare native installation metadata before entering the platform workflow.
pub(crate) async fn run(paths: &Paths) -> Result<()> {
    // Reject conflicting user/system installations before changing artifacts.
    crate::system::shared::services::artifacts::ensure_no_conflicting_install(paths)?;

    // Capture the currently selected service plan for rollback.
    let previous = crate::system::macos::services::definition::plan(paths)?;

    install(paths, previous).await
}

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;
