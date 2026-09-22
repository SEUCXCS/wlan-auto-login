//! Pause automatic login before logging out; restore the pause state on failure.
use crate::{config::Paths, control, service};
use anyhow::Result;
use cxxy_wlanlogin_lib::Client;

pub(super) async fn run(client: &Client, paths: &Paths) -> Result<()> {
    service::ensure_no_conflicting_install(paths)?;
    let _operation = control::operation_lock(paths).await?;
    let previous = control::State::load(paths)?;
    let mut paused = previous.clone();
    paused.paused = true;
    paused.save(paths)?;
    if let Err(error) = client.logout().await {
        previous.save(paths)?;
        return Err(error.into());
    }
    control::wake(paths).await;
    tracing::info!("已注销");
    Ok(())
}
