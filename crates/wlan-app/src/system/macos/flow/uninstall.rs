//! Add signed payload and helper cleanup to the common uninstall transaction.
use crate::infra::{cleanup, command::Runner, control, error::Result, paths::Paths};
use crate::system::{
    define::services::{Action, Manager, Plan, ServiceManager, ServiceState},
    macos::{
        runtime::host,
        services::{bundle, payload},
    },
    shared::services::hooks::UninstallHooks,
};
struct Hooks;
impl UninstallHooks for Hooks {
    async fn validate(&self, plan: &Plan) -> Result<()> {
        if let Some(app) = bundle::enclosing(&plan.executable)
            && app.exists()
        {
            payload::validate(&app).await?;
        }
        Ok(())
    }
    fn must_stop(&self, plan: &Plan, state: &ServiceState) -> bool {
        state.running
            || state.pending
            || (bundle::enclosing(&plan.executable).is_some() && state.registered)
    }
    async fn stop_registered(&self, plan: &Plan, runner: &impl Runner) -> Result<()> {
        // Native approval can persist without an actually loaded launchd job.
        plan.manager
            .manage(plan, Action::Stop, ServiceState::absent(), runner)
            .await
    }
    async fn stop_helpers(&self, paths: &Paths, plan: &Plan) -> Result<()> {
        if let Some(app) = bundle::enclosing(&plan.executable) {
            control::stop_managed(paths).await?;
            host::stop_helpers(paths.scope, &app).await?;
        }
        Ok(())
    }
    fn packaged_definition(&self, plan: &Plan) -> bool {
        matches!(plan.manager, Manager::MacosAppService { .. })
    }
    fn remove_payload(&self, plan: &Plan) -> Result<()> {
        match bundle::enclosing(&plan.executable) {
            Some(app) => payload::remove(&app),
            None => cleanup::remove_file(&plan.executable),
        }
    }
    fn runtime_files(&self, paths: &Paths, plan: &Plan) -> Vec<&'static str> {
        if bundle::enclosing(&plan.executable).is_none() {
            return Vec::new();
        }
        let mut files = vec!["managed.lock"];
        if paths.scope == crate::infra::scope::Scope::User {
            files.extend([
                "wifi-foreground.lock",
                "wifi-service.lock",
                "wifi-foreground.sock",
                "wifi-service.sock",
            ]);
        }
        files
    }
}

pub(crate) async fn run(paths: &Paths, purge: bool) -> Result<()> {
    // Common ordering owns locks, stop barriers, unregistration and scope cleanup.
    crate::system::shared::flow::uninstall::run_with(paths, purge, &Hooks).await
}
