//! Credential prerequisites and installation, independent of input presentation.
use super::login::LoginRequest;
use crate::infra::{
    error::{Message, Result},
    paths::Paths,
};
use crate::system::{
    credentials::Store,
    shared::services,
    wifi::{self, WifiConnection},
};
pub fn validate_scope(paths: &Paths) -> Result<()> {
    services::ensure_no_conflicting_install(paths)
}
pub async fn prepare(
    request: &LoginRequest,
    refresh: bool,
    wifi: &impl WifiConnection,
    store: &impl Store,
) -> Result<bool> {
    if let Some(notice) = wifi::wifi_state(wifi).await.notice() {
        tracing::warn!(
            "{}",
            Message::NoticeServiceInstallationCanContinueAndAuthentication { notice }
        );
    }
    let saved = store.load().await?;
    let required = saved.is_none()
        || refresh
        || request.username.is_some()
        || request.password.is_some()
        || request.credential_store.is_some();
    if required {
        tracing::warn!(
            "{}",
            Message::ServiceInstallationRequiresSavedCredentialsRememberMe
        );
    }
    Ok(required)
}
pub async fn run(paths: &Paths) -> Result<()> {
    services::install(paths).await
}
