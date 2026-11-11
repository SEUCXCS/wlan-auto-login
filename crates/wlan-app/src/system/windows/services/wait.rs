//! Task Scheduler and SCM stop barriers.
use crate::infra::{
    command::Runner,
    error::{Message, Result},
};
use crate::system::define::services::{Manager, ServiceManager, ServiceState};
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

#[cfg(target_os = "windows")]
pub(crate) async fn wait_task_started(
    manager: &Manager,
    runner: &impl Runner,
) -> Result<ServiceState> {
    use crate::infra::error::Context;
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        loop {
            let state = manager.query(runner).await?;
            crate::infra::error::ensure!(state.registered, Message::ServiceRegistrationRemoved);
            crate::infra::error::ensure!(
                state.enabled == Some(true),
                "Task enabled state is unavailable or disabled after registration"
            );
            if state.running || state.pending {
                return Ok(state);
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await
    .context("Timed out waiting for task start; installation will be rolled back")?
}
