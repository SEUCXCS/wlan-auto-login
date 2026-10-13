//! Extension points for native extra steps; the uninstall sequence stays shared.
use crate::infra::{
    cleanup,
    command::Runner,
    error::{Context, Message, Result},
    paths::Paths,
};
use crate::system::define::services::{Plan, ServiceState};

pub(crate) trait UninstallHooks {
    async fn validate(&self, _plan: &Plan) -> Result<()> {
        Ok(())
    }
    fn must_stop(&self, _plan: &Plan, state: &ServiceState) -> bool {
        state.running || state.pending
    }
    async fn stop_registered(&self, plan: &Plan, runner: &impl Runner) -> Result<()> {
        for command in &plan.stop {
            super::steps::execute_optional(runner, command).await?;
        }
        Ok(())
    }
    async fn stop_helpers(&self, _paths: &Paths, _plan: &Plan) -> Result<()> {
        Ok(())
    }
    fn packaged_definition(&self, _plan: &Plan) -> bool {
        false
    }
    fn remove_payload(&self, plan: &Plan) -> Result<()> {
        cleanup::remove_file(&plan.executable)
            .context(Message::ServiceUninstalledButInstalledExecutableCouldNotBe)
    }
    fn runtime_files(&self, _paths: &Paths, _plan: &Plan) -> Vec<&'static str> {
        Vec::new()
    }
}
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) struct DefaultHooks;
#[cfg(any(not(target_os = "macos"), test))]
impl UninstallHooks for DefaultHooks {}
