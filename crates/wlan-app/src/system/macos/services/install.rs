//! Install a verified app as one transaction, including Keychain identity checks.
use super::payload::{self, Payload};
use crate::infra::{
    cleanup,
    command::{Invocation, SystemRunner},
    control,
    error::{Context, Message, Result},
    paths::{self, Paths},
    scope::Scope,
};
use crate::system::{
    define::services::{Action, Plan, ServiceManager},
    macos::{bundle, credentials::access, host},
};
use std::{fs, path::Path};

pub async fn install(paths: &Paths, plan: Plan) -> Result<()> {
    crate::infra::error::ensure!(
        paths.scope == plan.scope,
        crate::infra::error::Message::ServicePlanDoesNotMatchTheSelected
    );
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
    let source = bundle::source()?;
    payload::validate(&source).await?;
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
    let previous = match fs::read(&plan.definition) {
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
    fs::create_dir_all(
        plan.definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?,
    )?;
    let state = plan.manager.query(&SystemRunner).await?;
    stop(paths, &plan).await?;
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
        transaction.publish()?;
        // Application root is public code, while state/secrets stay private.
        if paths.scope == Scope::System {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                destination.parent().context(Message::InstallationPathHasNoParentDirectory)?,
                fs::Permissions::from_mode(0o755),
            )?;
        }
        authorize(paths.scope, &plan.executable).await?;
        verify_credentials(&plan).await?;
        // Authorize the newly installed identity, which can differ after ad-hoc signing.
        if let Err(error) = host::prepare(access::interaction_allowed()).await {
            if paths.scope == Scope::System && !access::interaction_allowed() {
                tracing::warn!(%error, "System service will wait for an authorized GUI-session Wi-Fi host");
            } else { return Err(error); }
        }
        crate::system::shared::services::install::write_definition(
            &plan.definition,
            plan.content.as_bytes(),
        )?;
        for command in &plan.start {
            command.run().await?;
        }
        super::runtime::wait_ready(paths, &plan).await?;
        Ok::<_, crate::infra::error::Error>(())
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
        if failures.is_empty() {
            if let Err(e) = transaction.restore() {
                failures.push(e.to_string());
            }
            let restored = match &previous {
                Some(bytes) => crate::system::shared::services::install::write_definition(
                    &plan.definition,
                    bytes,
                ),
                None => cleanup::remove_file(&plan.definition),
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
                if let Err(e) = authorize(paths.scope, executable).await {
                    failures.push(e.to_string());
                }
                if transaction.existed()
                    && let Err(e) = verify_credentials(&plan).await
                {
                    failures.push(e.to_string());
                }
            }
            if failures.is_empty() && state.registered {
                for command in &plan.start {
                    if let Err(e) = command.run().await {
                        failures.push(e.to_string());
                    }
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
    tracing::info!(
        scope = paths.scope.label(),
        "Signed application installed and managed daemon ready"
    );
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
async fn verify_credentials(plan: &Plan) -> Result<()> {
    Invocation::new(
        &plan.executable,
        [
            if plan.scope == Scope::System {
                "--system"
            } else {
                "--user"
            },
            "macos-credential-check",
        ],
    )
    .run()
    .await
    .context(Message::InstalledCredentialCheckFailed)
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
