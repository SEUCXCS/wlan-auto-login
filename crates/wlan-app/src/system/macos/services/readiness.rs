use crate::infra::{
    control,
    error::{Context, Message, Result, ensure},
    paths::Paths,
};
use crate::system::define::services::{Plan, ServiceManager};
use std::time::Duration;

pub async fn wait_ready(paths: &Paths, plan: &Plan) -> Result<()> {
    let state = plan
        .manager
        .query(&crate::infra::command::SystemRunner)
        .await?;
    ensure!(
        state.enabled != Some(false),
        Message::MacosBackgroundApprovalRequired
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if control::managed_running(paths).await {
                return Ok::<_, crate::infra::error::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context(Message::ManagedAppNotReady)??;
    // User daemon startup creates its signed host before publishing control IPC.
    // A root service is ready even before GUI login: SSID must then wait for a user.
    ensure!(plan.executable.is_file(), Message::InstallationIncomplete);
    Ok(())
}
