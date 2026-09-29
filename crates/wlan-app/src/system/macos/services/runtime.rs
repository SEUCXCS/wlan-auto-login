//! Control the actual Launch Services app after removing launchd's wrapper.
use crate::infra::{
    command::Runner,
    control,
    error::{Context, Message, Result, ensure},
    paths::Paths,
};
use crate::system::define::services::{Action, Plan, ServiceManager, ServiceState};
use std::time::Duration;

/// Acquire before consulting launchd. After bootout, a delayed Launch Services
/// child must fail this check before creating a host or a daemon endpoint.
pub async fn guard(paths: &Paths) -> Result<std::fs::File> {
    let guard = control::managed_lock(paths)?;
    let plan = super::definition::plan(paths)?;
    let state = plan
        .manager
        .query(&crate::infra::command::SystemRunner)
        .await?;
    ensure!(state.registered, Message::ServiceNotRegistered);
    Ok(guard)
}

pub async fn manage(
    paths: &Paths,
    plan: &Plan,
    action: Action,
    mut state: ServiceState,
    runner: &impl Runner,
) -> Result<()> {
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
    wait_ready(paths, plan).await
}

pub async fn wait_ready(paths: &Paths, plan: &Plan) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if control::managed_running(paths).await {
                return Ok::<_, crate::infra::error::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("Service registration succeeded but the managed application did not become ready")??;
    // User service startup creates its signed host before publishing control IPC.
    // A root service is ready even before GUI login: SSID must then wait for a user.
    ensure!(plan.executable.is_file(), Message::InstallationIncomplete);
    Ok(())
}
