//! Small reusable service steps, shared by every operation and platform.
use crate::infra::{
    command::{Invocation, Runner},
    error::Result,
};

pub(crate) async fn execute(runner: &impl Runner, command: Invocation) -> Result<()> {
    runner.output(&command).await?.check(&command)
}

/// Signed native commands validate the current protocol at the execution boundary.
pub(crate) async fn execute_optional(runner: &impl Runner, command: &Invocation) -> Result<()> {
    crate::system::native::services::execute_optional(runner, command).await
}
