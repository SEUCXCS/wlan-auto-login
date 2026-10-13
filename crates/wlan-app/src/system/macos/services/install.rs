//! Install a verified app as one transaction, including Keychain identity checks.
use super::payload::{self, Payload};
use crate::infra::{
    cleanup,
    command::{Invocation, Runner, SystemRunner},
    control,
    error::{Context, Message, Result},
    paths::{self, Paths},
    scope::Scope,
};
use crate::system::{
    define::{
        permission::PermissionStatus,
        services::{Action, Plan, ServiceManager},
    },
    macos::{bundle, credentials::access, host},
};
use std::{fs, future::Future, path::Path};

pub async fn install(paths: &Paths, previous_plan: Plan) -> Result<()> {
    crate::infra::error::ensure!(
        paths.scope == previous_plan.scope,
        crate::infra::error::Message::ServicePlanDoesNotMatchTheSelected
    );
    let plan = super::definition::native_plan(paths)?;
    let legacy_plan = super::definition::legacy_plan(paths)?;
    if paths.scope == Scope::System {
        prepare_system_directories()?;
    }
    let _lock = control::service_lock(paths)?;
    crate::system::shared::services::artifacts::assert_owned(&plan.definition)?;
    crate::system::shared::services::artifacts::assert_owned(&legacy_plan.definition)?;
    cleanup::validate_directory(
        plan.definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;
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
    let destination = paths::macos_bundle(paths.scope)?;
    let mut transaction = Payload::stage(&source, &destination)?;
    payload::validate(&transaction.staged()).await?;
    if destination.exists() {
        payload::validate(&destination).await?;
    }
    let legacy = destination
        .parent()
        .context(Message::InstallationPathHasNoParentDirectory)?
        .join("bin")
        .join(crate::infra::identity::DAEMON_BINARY);
    let previous = match fs::read(&legacy_plan.definition) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let legacy_owned = previous.as_ref().is_some_and(|bytes| {
        String::from_utf8_lossy(bytes).contains(&crate::system::shared::services::artifacts::xml(
            &legacy.to_string_lossy(),
        ))
    });
    if let Some(bytes) = &previous {
        fs::write(transaction.recovery_path("old.plist"), bytes)?;
    }
    let state = previous_plan.manager.query(&SystemRunner).await?;
    stop(paths, &previous_plan).await?;
    let outcome = async {
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
        cleanup::remove_file(&legacy_plan.definition)?;
        transaction.publish()?;
        // Application root is public code, while state/secrets stay private.
        if paths.scope == Scope::System {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                destination.parent().context(Message::InstallationPathHasNoParentDirectory)?,
                fs::Permissions::from_mode(0o755),
            )?;
        }
        prepare_credentials(paths.scope, &plan.executable, CheckFormat::Current).await?;
        // Authorize the newly installed identity, which can differ after ad-hoc signing.
        if let Err(error) = host::prepare(access::interaction_allowed()).await {
            if paths.scope == Scope::System && !access::interaction_allowed() {
                tracing::warn!(%error, "System service will wait for an authorized GUI-session Wi-Fi host");
            } else { return Err(error); }
        }
        // Packaged definitions were signed by xtask. Installation never rewrites them.
        for command in &plan.start {
            super::app_service::output(command, &SystemRunner).await?;
        }
        let registered = plan.manager.query(&SystemRunner).await?;
        if registered.registered && registered.enabled == Some(false) {
            return Ok::<_, crate::infra::error::Error>(false);
        }
        super::runtime::wait_ready(paths, &plan).await?;
        Ok(true)
    }
    .await;
    if let Err(error) = outcome {
        // Never restore files underneath a process which failed to stop.
        let stopped = stop(paths, &plan).await;
        let mut failures = Vec::new();
        if let Err(e) = stopped {
            failures.push(e.to_string());
        }
        if failures.is_empty()
            && let Err(e) = host::stop_helpers(paths.scope, &destination).await
        {
            failures.push(e.to_string());
        }
        if failures.is_empty()
            && let Err(e) = unregister(&plan).await
        {
            failures.push(e.to_string());
        }
        if failures.is_empty() {
            if let Err(e) = transaction.restore() {
                failures.push(e.to_string());
            }
            let restored = match &previous {
                Some(bytes) => crate::system::shared::services::install::write_definition(
                    &legacy_plan.definition,
                    bytes,
                ),
                None => cleanup::remove_file(&legacy_plan.definition),
            };
            if let Err(e) = restored {
                failures.push(e.to_string());
            }
            if failures.is_empty() {
                let original = std::env::current_exe()?;
                let executable = if transaction.existed() {
                    &plan.executable
                } else if legacy_owned {
                    &legacy
                } else {
                    &original
                };
                let restored = if transaction.existed() {
                    prepare_credentials(paths.scope, executable, CheckFormat::Previous).await
                } else if legacy_owned {
                    // A legacy executable may predate the installed-check protocol.
                    authorize(paths.scope, executable).await
                } else {
                    prepare_credentials(paths.scope, executable, CheckFormat::Current).await
                };
                if let Err(e) = restored {
                    failures.push(e.to_string());
                }
            }
            if failures.is_empty() && state.registered {
                for command in &previous_plan.start {
                    if let Err(e) = super::app_service::output(command, &SystemRunner).await {
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
    let ready = outcome?;
    transaction.commit();
    // Remove only the same-ID legacy loose executable identified by the old plist.
    if legacy_owned {
        cleanup::remove_file(&legacy)?;
        cleanup::prune_empty_directories(
            legacy
                .parent()
                .context(Message::InstallationPathHasNoParentDirectory)?,
        )?;
    }
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
            super::app_service::output(command, &SystemRunner).await?;
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckFormat {
    Current,
    Previous,
}

async fn prepare_credentials(scope: Scope, executable: &Path, format: CheckFormat) -> Result<()> {
    ensure_credential_access(
        || credential_status(scope, executable, &SystemRunner, format),
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
    format: CheckFormat,
) -> Result<PermissionStatus> {
    let current = format == CheckFormat::Current
        || super::app_service::current_protocol(executable, scope == Scope::System)?;
    let flag = if scope == Scope::System {
        "--system"
    } else {
        "--user"
    };
    let command = if current {
        Invocation::new(
            executable,
            [flag, "permission", "check", "macos.keychain", "--json"],
        )
    } else {
        Invocation::new(executable, [flag, "macos-credential-check"])
    };
    let output = runner
        .output(&command)
        .await
        .context(Message::InstalledCredentialCheckFailed)?;
    output
        .check(&command)
        .context(Message::InstalledCredentialCheckFailed)?;
    // The previous installed checker reported success only after a silent read,
    // using its exit status with no stdout. Accept that proof during rollback.
    if !current && format == CheckFormat::Previous && output.stdout.trim().is_empty() {
        return Ok(PermissionStatus::Granted);
    }
    if current {
        serde_json::from_str::<crate::system::define::permission::PermissionReport>(&output.stdout)
            .context(Message::InstalledCredentialCheckProtocol)?
            .into_status(scope.label(), "macos.keychain")
    } else {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PreviousCheck {
            version: u32,
            status: PermissionStatus,
        }
        let result = serde_json::from_str::<PreviousCheck>(&output.stdout)
            .context(Message::InstalledCredentialCheckProtocol)?;
        crate::infra::error::ensure!(
            result.version == 1,
            Message::InstalledCredentialCheckProtocol
        );
        Ok(result.status)
    }
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
                credential_status(
                    Scope::User,
                    Path::new("/isolated/app"),
                    &runner,
                    CheckFormat::Current
                )
                .await
                .is_err()
            );
        }
        let runner = FakeRunner::new([(
            true,
            r#"{"version":1,"scope":"system","permissions":[{"id":"macos.keychain","status":"Granted","error":null}]}"#,
        )]);
        assert_eq!(
            credential_status(
                Scope::System,
                Path::new("/isolated/app"),
                &runner,
                CheckFormat::Current
            )
            .await?,
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
    async fn rollback_accepts_previous_silent_checker_success_but_never_failure() -> Result<()> {
        let runner = FakeRunner::new([(true, "")]);
        assert_eq!(
            credential_status(
                Scope::User,
                Path::new("/isolated/old-app"),
                &runner,
                CheckFormat::Previous
            )
            .await?,
            PermissionStatus::Granted
        );
        let runner = FakeRunner::new([(false, "")]);
        assert!(
            credential_status(
                Scope::User,
                Path::new("/isolated/old-app"),
                &runner,
                CheckFormat::Previous
            )
            .await
            .is_err()
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
