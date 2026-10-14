//! Collect installation, native registration, version and runtime diagnostics.
use crate::infra::{
    command::{Invocation, Runner, SystemRunner},
    error::{Context, Message, Result},
    paths::Paths,
    scope::Scope,
};
use crate::system::{
    define::services::{Artifacts, InstallationReport, Plan, ServiceManager, ServiceState},
    shared::services::artifacts::plan,
};

pub(crate) trait Backend {
    fn query(
        &self,
        paths: Option<&Paths>,
        plan: &Plan,
        runner: &impl Runner,
    ) -> impl std::future::Future<Output = Result<ServiceState>>;
}
pub(crate) struct DefaultBackend;
impl Backend for DefaultBackend {
    async fn query(
        &self,
        _paths: Option<&Paths>,
        plan: &Plan,
        runner: &impl Runner,
    ) -> Result<ServiceState> {
        plan.manager.query(runner).await
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) async fn run(paths: &Paths, discover: bool) -> Result<Vec<InstallationReport>> {
    run_with(paths, discover, &DefaultBackend).await
}

pub(crate) async fn run_with(
    paths: &Paths,
    discover: bool,
    backend: &impl Backend,
) -> Result<Vec<InstallationReport>> {
    // Discover only the requested scopes using their native installation plans.
    let mut plans = vec![plan(paths)?];
    if discover && paths.scope == Scope::User {
        plans.push(plan(&Paths::new(Scope::System)?)?);
    }

    // Read artifacts, query native state, then collect the safe runtime snapshot.
    let mut reports = Vec::new();
    for plan in plans {
        let scope_paths = Paths::new(plan.scope)?;
        let mut report = inspect_with(&plan, &SystemRunner, Some(&scope_paths), backend).await?;

        // Authenticated status IPC reports only state; it never opens credential storage.
        if report
            .state
            .as_ref()
            .is_some_and(|r| r.as_ref().is_ok_and(|s| s.running))
        {
            report.runtime = Some(crate::infra::control::status(&scope_paths).await);
        }

        // Return only installations with observable artifacts.
        if report.files.any() {
            reports.push(report);
        }
    }
    Ok(reports)
}

#[cfg(test)]
pub(crate) async fn inspect(plan: &Plan, runner: &impl Runner) -> Result<InstallationReport> {
    inspect_with(plan, runner, None, &DefaultBackend).await
}
async fn inspect_with(
    plan: &Plan,
    runner: &impl Runner,
    paths: Option<&Paths>,
    backend: &impl Backend,
) -> Result<InstallationReport> {
    // Artifact inspection validates ownership independently of native registration.
    let files = Artifacts::read(plan)?;

    // A missing install must not execute a native manager or installed program.
    let state = if files.any() {
        Some(backend.query(paths, plan, runner).await)
    } else {
        None
    };
    let registration_missing = matches!(&state, Some(Ok(state)) if !plan.manager.installed(state));

    // Execute only --version, and only for a complete installation.
    let version = if files.complete() {
        Some(installed_version(plan, runner).await)
    } else {
        None
    };
    Ok(InstallationReport {
        scope: plan.scope,
        manager_label: plan.manager.label(),
        service_name: plan.manager.name().into(),
        executable: plan.executable.clone(),
        definition: plan.definition.clone(),
        files,
        registration_missing,
        state,
        version,
        runtime: None,
    })
}
async fn installed_version(plan: &Plan, runner: &impl Runner) -> Result<String> {
    // Only --version is executed, so this never enters the daemon/login code path.
    let command = Invocation::new(&plan.executable, ["--version"]);
    let result = runner.output(&command).await?;
    result.check(&command)?;
    let version = result
        .stdout
        .trim()
        .strip_prefix(crate::infra::identity::APP_NAME)
        .and_then(|value| value.strip_prefix(' '))
        .filter(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ".-+".contains(ch))
        })
        .context(Message::InstalledExecutableReturnedAnUnrecognizedVersion)?;
    Ok(version.into())
}
