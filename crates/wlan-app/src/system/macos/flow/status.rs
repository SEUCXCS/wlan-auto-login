//! Refine launchd wrapper status using the authenticated daemon endpoint.
use crate::infra::{error::Result, scope::Scope};
#[cfg(test)]
pub(crate) use crate::system::shared::flow::status::inspect;
pub(crate) use crate::system::shared::flow::status::run;
use crate::system::{
    define::services::ServiceState,
    shared::flow::{
        hooks::Hooks,
        status::{StatusContext, StatusHooks, Step},
    },
};

pub(crate) struct NativeHooks;
impl Hooks<Step, StatusContext<'_>> for NativeHooks {}
impl StatusHooks for NativeHooks {
    async fn after_query(
        &self,
        context: &StatusContext<'_>,
        state: &mut ServiceState,
    ) -> Result<()> {
        if context.plan.scope == Scope::User
            && let Some(paths) = context.paths
            && crate::system::macos::services::bundle::enclosing(&context.plan.executable).is_some()
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
        Ok(())
    }
}
