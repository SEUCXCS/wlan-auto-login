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
    let stop = crate::system::native::runtime::daemon::stop(&mut shutdown);
    until_stopped(run_inner(client, paths, managed), stop).await?;
    tracing::info!("{}", Message::DaemonStopped);
    Ok(())
}
async fn run_inner(client: &Client, paths: &Paths, managed: bool) -> Result<()> {
    ensure_scope(paths)?;
    stores::prepare_daemon()?;
    let _singleton = control::singleton(paths)?;
    let (server, mut commands) = control::serve_with_mode(paths, managed).await?;
    let wake = tokio::sync::Notify::new();
    let (session, readiness) =
        tokio::sync::watch::channel(crate::system::define::credentials::SessionState::default());
    until_stopped(
        runtime::run_loop(client, paths, &wake, readiness, &server.status),
        runtime::control_stop_with_session(&mut commands, &wake, &session),
    )
    .await
}
