//! SMAppService registration, wrapper liveness and managed-daemon readiness differ.
use crate::infra::{command::Runner, error::Result, paths::Paths, scope::Scope};
use crate::system::define::services::{Plan, ServiceState};

struct NativeBackend;
impl crate::system::shared::flow::status::Backend for NativeBackend {
    async fn query(
        &self,
        paths: Option<&Paths>,
        plan: &Plan,
        runner: &impl Runner,
    ) -> Result<ServiceState> {
        use crate::system::shared::flow::status::DefaultBackend;
        // Registration and the managed IPC endpoint establish different readiness facts.
        let mut state = DefaultBackend.query(paths, plan, runner).await?;
        if plan.scope == Scope::User
            && let Some(paths) = paths
            && crate::system::macos::services::bundle::enclosing(&plan.executable).is_some()
        {
            let actual = crate::infra::control::managed_pid(paths).await;
            let wrapper_running = state.running;
            state.running = actual.is_some();
            state.pid = actual;
            if wrapper_running && !state.running {
                state.state = "waiting for managed daemon readiness".into();
                state.pending = true;
            }
        }
        Ok(state)
    }
}

pub(crate) async fn run(
    paths: &Paths,
    discover: bool,
) -> Result<Vec<crate::system::define::services::InstallationReport>> {
    crate::system::shared::flow::status::run_with(paths, discover, &NativeBackend).await
}

#[cfg(test)]
pub(crate) use crate::system::shared::flow::status::inspect;
