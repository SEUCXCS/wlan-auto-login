//! Small reusable service steps, shared by every operation and platform.
use crate::infra::{
    command::{Invocation, Runner},
    error::Result,
};
use crate::system::define::services::{ServiceManager, ServiceState};
#[cfg(any(target_os = "windows", test))]
use crate::{infra::error::Message, system::define::services::Manager};
use crate::{
    infra::paths::Paths,
    system::define::services::{Action, Plan},
};

pub(crate) async fn apply(
    _paths: &Paths,
    plan: &Plan,
    action: Action,
    state: ServiceState,
    runner: &impl Runner,
) -> Result<()> {
    // Only the registered service is controlled; a foreground daemon is independent.
    plan.manager.manage(plan, action, state, runner).await
}

pub(crate) async fn execute(runner: &impl Runner, command: Invocation) -> Result<()> {
    runner.output(&command).await?.check(&command)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) async fn wait_task_stopped(
    manager: &Manager,
    runner: &impl Runner,
) -> Result<ServiceState> {
    use crate::infra::error::Context;
    use std::time::Duration;
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let state = manager.query(runner).await?;
            if !state.running && !state.pending {
                return Ok(state);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context(Message::TimedOutWaitingForServiceStopNoNew)?
}

/// Signed native commands validate the current protocol at the execution boundary.
pub(crate) async fn execute_optional(runner: &impl Runner, command: &Invocation) -> Result<()> {
    #[cfg(any(target_os = "macos", test))]
    {
        crate::system::macos::services::app_service::output(command, runner).await?;
    }
    #[cfg(not(any(target_os = "macos", test)))]
    {
        let output = runner.output(command).await?;
        if !command.optional {
            output.check(command)?;
        }
    }
    Ok(())
}
