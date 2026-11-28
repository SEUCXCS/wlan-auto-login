//! Add signed payload and helper cleanup to the common uninstall transaction.
use crate::infra::{
    cleanup,
    command::Runner,
    control,
    error::{Error, Result},
    paths::Paths,
};
use crate::system::{
    define::services::{Action, Manager, Plan, ServiceManager, ServiceState},
    macos::{
        runtime::host,
        services::{bundle, payload},
    },
    shared::flow::{
        hooks::Hooks,
        uninstall::{Step, UninstallContext, UninstallHooks},
    },
};
pub(crate) struct NativeHooks;
impl Hooks<Step, UninstallContext<'_>> for NativeHooks {
    async fn after(
        &self,
        step: Step,
        context: &UninstallContext<'_>,
        result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        if result.is_err() {
            return Ok(());
        }
        let Some(app) = bundle::enclosing(&context.plan.executable) else {
            return Ok(());
        };
        match step {
            Step::Validate if app.exists() => payload::validate(&app).await?,
            Step::Stop => {
                control::stop_managed(context.paths).await?;
                host::stop_helpers(context.paths.scope, &app).await?;
            }
            _ => {}
        }
        Ok(())
    }
}
impl UninstallHooks for NativeHooks {
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
        if paths.scope == crate::infra::scope::Scope::User {
            vec!["wifi-foreground.sock", "wifi-service.sock"]
        } else {
            Vec::new()
        }
    }
}
