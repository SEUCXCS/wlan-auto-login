use crate::infra::error::Result;
use crate::system::{credentials::ScopedStore, define::credentials::BackgroundRead};

pub(crate) async fn run(
    store: &ScopedStore,
    session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
) -> Result<BackgroundRead> {
    // Background policy never permits an interactive unlock request.
    store.load_background(session).await
}

pub(crate) use crate::system::shared::credentials::diagnostics::background as failure;

pub(crate) async fn validate(
    paths: &crate::infra::paths::Paths,
    permit: &crate::system::define::credentials::CredentialPermit,
    session: &tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
) -> Result<crate::system::define::credentials::Validity> {
    // A final pre-login checkpoint has its own deadline and credential diagnostics.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::system::shared::credentials::validation::validate(paths, permit, session),
    )
    .await
    .map_err(Into::into)
    .and_then(|result| result)
    .map_err(|error| {
        crate::system::shared::credentials::diagnostics::at(
            error,
            crate::infra::error::credentials::Stage::Session,
        )
    })
}
