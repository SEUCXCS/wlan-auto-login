//! Small reusable service steps, shared by every operation and platform.
use crate::infra::{
    command::{Invocation, Runner},
    error::Result,
};
use crate::system::define::services::{ServiceManager, ServiceState};
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

/// Signed native commands validate the current protocol at the execution boundary.
pub(crate) async fn execute_optional(runner: &impl Runner, command: &Invocation) -> Result<()> {
    crate::system::native::services::execute_optional(runner, command).await
}
