//! Collect installation and manager state without writing to a terminal.
use crate::infra::{
    command::{Invocation, Runner, SystemRunner},
    error::{Context, Message, Result},
    paths::Paths,
    scope::Scope,
};
use crate::system::{
    define::services::{Plan, ServiceManager, ServiceState},
    shared::services::artifacts::{Artifacts, plan},
};
use std::path::PathBuf;

pub struct InstallationReport {
    pub scope: Scope,
    pub manager_label: &'static str,
    pub service_name: String,
    pub executable: PathBuf,
    pub definition: PathBuf,
    pub files: Artifacts,
    pub registration_missing: bool,
    pub state: Option<Result<ServiceState>>,
    pub version: Option<Result<String>>,
}

pub async fn run(paths: &Paths, discover: bool) -> Result<Vec<InstallationReport>> {
    let mut plans = vec![plan(paths)?];
    if discover && paths.scope == Scope::User {
        plans.push(plan(&Paths::new(Scope::System)?)?);
    }
    let mut reports = Vec::new();
    for plan in plans {
        let report = inspect(&plan, &SystemRunner).await?;
        #[cfg(target_os = "macos")]
        let report = {
            let mut report = report;
            if plan.scope == Scope::User
                && crate::system::macos::bundle::enclosing(&plan.executable).is_some()
                && let Some(Ok(state)) = &mut report.state
            {
                let actual = crate::infra::control::managed_pid(&Paths::new(plan.scope)?).await;
                let wrapper_running = state.running;
                state.running = actual.is_some();
                state.pid = actual;
                if wrapper_running && !state.running {
                    state.state = "waiting for managed daemon readiness".into();
                    state.pending = true;
                }
            }
            report
        };
        if report.files.any() {
            reports.push(report);
        }
    }
    Ok(reports)
}

pub fn check(reports: &[InstallationReport]) -> Result<()> {
    let errors = reports
        .iter()
        .filter_map(|report| report.state.as_ref().and_then(|state| state.as_ref().err()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    crate::infra::error::ensure!(
        errors.is_empty(),
        Message::FailedToDetermineServiceStatus {
            value0: errors.join("; ")
        }
    );
    Ok(())
}

pub(crate) async fn inspect(plan: &Plan, runner: &impl Runner) -> Result<InstallationReport> {
    let files = Artifacts::read(plan)?;
    let state = if files.any() {
        Some(plan.manager.query(runner).await)
    } else {
        None
    };
    let registration_missing = matches!(&state, Some(Ok(state)) if !plan.manager.installed(state));
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
