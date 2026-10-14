//! Credential prerequisites and installation, independent of input presentation.
use crate::app::controller::login::LoginRequest;
use crate::infra::{
    error::{Message, Result},
    paths::Paths,
};
use crate::system::{
    credentials::Store,
    wifi::{self, WifiConnection},
};
pub fn validate_scope(paths: &Paths) -> Result<()> {
    crate::system::flow::install::validate_scope(paths)
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
    let required = refresh
        || request.username.is_some()
        || request.password.is_some()
        || request.credential_store.is_some()
        || crate::system::flow::credentials::load::run(store)
            .await?
            .is_none();
    if required {
        tracing::warn!(
            "{}",
            Message::ServiceInstallationRequiresSavedCredentialsRememberMe
        );
    }
    Ok(required)
}
pub async fn run(paths: &Paths) -> Result<()> {
    crate::system::flow::install::run(paths).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wlanlogin_lib::Credentials;

    struct Offline;
    impl WifiConnection for Offline {
        async fn target_session(&self) -> Result<Option<String>> {
            Ok(None)
        }
    }

    struct InaccessibleStore;
    impl Store for InaccessibleStore {
        async fn load(&self) -> Result<Option<Credentials>> {
            crate::infra::error::bail!("credentials need interaction")
        }
        async fn save(&self, _: Credentials) -> Result<()> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn credential_refresh_does_not_read_the_keychain_before_login_preparation() -> Result<()>
    {
        let explicit = LoginRequest {
            username: Some("student".into()),
            password: Some("secret".into()),
            ..Default::default()
        };
        assert!(prepare(&explicit, false, &Offline, &InaccessibleStore).await?);
        assert!(prepare(&LoginRequest::default(), true, &Offline, &InaccessibleStore).await?);
        assert!(
            prepare(
                &LoginRequest::default(),
                false,
                &Offline,
                &InaccessibleStore
            )
            .await
            .is_err()
        );
        Ok(())
    }
}
