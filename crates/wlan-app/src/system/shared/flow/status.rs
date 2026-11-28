//! Collect installation, native registration, version and runtime diagnostics.
use super::hooks::{Hooks, NoHooks, step};
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

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Inspect,
    Query,
    Version,
    Runtime,
}

pub(crate) struct StatusContext<'a> {
    pub paths: Option<&'a Paths>,
    pub plan: &'a Plan,
}

pub(crate) trait StatusHooks: for<'a> Hooks<Step, StatusContext<'a>> {
    async fn after_query(
        &self,
        _context: &StatusContext<'_>,
        _state: &mut ServiceState,
    ) -> Result<()> {
        Ok(())
    }
}
impl StatusHooks for NoHooks {}

pub(crate) async fn run(paths: &Paths, discover: bool) -> Result<Vec<InstallationReport>> {
    let hooks = super::NativeStatus;
    // Discover only the requested scopes using their native installation plans.
    let mut plans = vec![plan(paths)?];
    if discover && paths.scope == Scope::User {
        plans.push(plan(&Paths::new(Scope::System)?)?);
    }

    // Read artifacts, query native state, then collect the safe runtime snapshot.
    let mut reports = Vec::new();
    for plan in plans {
        let scope_paths = Paths::new(plan.scope)?;
        let context = StatusContext {
            paths: Some(&scope_paths),
            plan: &plan,
        };
        let mut report = inspect_with(&context, &SystemRunner, &hooks).await?;

        // Authenticated status IPC reports only state; it never opens credential storage.
        if let Some(paths) = context.paths
            && report
                .state
                .as_ref()
                .is_some_and(|state| state.as_ref().is_ok_and(|state| state.running))
        {
            report.runtime = Some(
                step(
                    &hooks,
                    Step::Runtime,
                    &context,
                    crate::infra::control::status(paths),
                )
                .await,
            );
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
    inspect_with(&StatusContext { plan, paths: None }, runner, &NoHooks).await
}
async fn inspect_with(
    context: &StatusContext<'_>,
    runner: &impl Runner,
    hooks: &impl StatusHooks,
) -> Result<InstallationReport> {
    let plan = context.plan;
    let files = step(hooks, Step::Inspect, context, async {
        Artifacts::read(plan)
    })
    .await?;
    let state = if files.any() {
        Some(
            step(hooks, Step::Query, context, async {
                let mut state = plan.manager.query(runner).await?;
                hooks.after_query(context, &mut state).await?;
                Ok(state)
            })
            .await,
        )
    } else {
        None
    };
    let registration_missing = matches!(&state, Some(Ok(state)) if !plan.manager.installed(state));

    // Execute only --version, and only for a complete installation.
    let version = if files.complete() {
        Some(
            step(
                hooks,
                Step::Version,
                context,
                installed_version(plan, runner),
            )
            .await,
        )
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
