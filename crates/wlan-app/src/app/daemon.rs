//! Own the daemon lifetime, singleton and cancellation sources.
mod check;
mod credentials;
mod policy;
mod runtime;
use crate::infra::{
    control,
    error::{Message, Result},
    paths::Paths,
    shutdown::{Shutdown, until_stopped},
};
use crate::system::credentials as stores;
use wlanlogin_lib::Client;
fn ensure_scope(paths: &Paths) -> Result<()> {
    // User daemons yield when a system installation exists. A boot service must
    // not inspect user homes (which systemd intentionally hides from it).
    if paths.scope == crate::infra::scope::Scope::User {
        crate::system::shared::services::ensure_no_conflicting_install(paths)?;
    }
    Ok(())
}
pub async fn run(client: &Client, paths: &Paths, managed: bool) -> Result<()> {
    let mut shutdown = Shutdown::new()?;
    let stop = async {
        #[cfg(windows)]
        {
            tokio::select! {
                result = shutdown.wait() => result,
                _ = crate::system::windows::services::runtime::stop_signal() => Ok(()),
            }
        }
        #[cfg(not(windows))]
        {
            shutdown.wait().await
        }
    };
    until_stopped(run_inner(client, paths, managed), stop).await?;
    tracing::info!("{}", Message::DaemonStopped);
    Ok(())
}
async fn run_inner(client: &Client, paths: &Paths, managed: bool) -> Result<()> {
    ensure_scope(paths)?;
    stores::prepare_daemon()?;
    let _singleton = control::singleton(paths)?;
    let (_server, mut commands) = control::serve_with_mode(paths, managed).await?;
    let wake = tokio::sync::Notify::new();
    until_stopped(
        runtime::run_loop(client, paths, &wake),
        runtime::control_stop(&mut commands, &wake),
    )
    .await
}
