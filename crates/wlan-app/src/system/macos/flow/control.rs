use crate::infra::{command::Runner, control, error::Result, paths::Paths};
use crate::system::define::services::{Action, Plan, ServiceManager, ServiceState};

pub(crate) async fn apply(
    paths: &Paths,
    plan: &Plan,
    action: Action,
    mut state: ServiceState,
    runner: &impl Runner,
) -> Result<()> {
    // Plans without a signed bundle use only the normalized manager contract.
    if crate::system::macos::services::bundle::enclosing(&plan.executable).is_none() {
        return crate::system::shared::services::steps::apply(paths, plan, action, state, runner)
            .await;
    }

    if matches!(action, Action::Stop | Action::Restart) {
        // bootout first, so KeepAlive cannot race the explicit app shutdown.
        plan.manager
            .manage(plan, Action::Stop, state, runner)
            .await?;
        control::stop_managed(paths).await?;
        state = ServiceState::absent();
        if matches!(action, Action::Stop) {
            return Ok(());
        }
    }
    plan.manager.manage(plan, action, state, runner).await?;
    crate::system::macos::services::readiness::wait_ready(paths, plan).await
}
