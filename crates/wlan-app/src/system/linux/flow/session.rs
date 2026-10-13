//! Optional hooks for desktops that do not manage graphical-session.target.
use crate::infra::{
    control,
    error::{Message, Result, ensure},
    paths::Paths,
    scope::Scope,
};

pub(crate) async fn ready(paths: &Paths) -> Result<()> {
    // Only a logged-in user can update the user activation environment.
    ensure!(paths.scope == Scope::User, Message::SessionNotReady);
    paths.scope.require_access()?;
    let session = crate::system::linux::runtime::session::import_current_session().await?;

    // The authenticated daemon keeps the hint in memory and revalidates it on use.
    control::session_ready(paths, session).await
}

pub(crate) async fn end(paths: &Paths) -> Result<()> {
    // Bind logout to this session; a later login has a different identity.
    ensure!(paths.scope == Scope::User, Message::SessionNotReady);
    paths.scope.require_access()?;
    let session = crate::system::linux::runtime::session::current_session_id().await?;
    control::session_end(paths, session).await
}
