//! Prepare saved credentials before installing the operating-system service.
use super::{cli::LoginArgs, login};
use crate::infra::error::Message;
use crate::infra::error::Result;
use crate::{infra::paths::Paths, service, system::credentials, system::wifi};
use cxxy_wlanlogin_lib::Client;

pub(super) async fn run(client: &Client, paths: &Paths, args: LoginArgs) -> Result<()> {
    service::ensure_no_conflicting_install(paths)?;
    let wifi = wifi::WifiProbe::new().await;
    if let Some(notice) = wifi::wifi_state(&wifi).await.notice() {
        tracing::warn!(
            "{}",
            Message::NoticeServiceInstallationCanContinueAndAuthentication {
                notice: (notice).to_string()
            }
        );
    }
    let store = credentials::ScopedStore::new(paths, args.credential_store)?;
    let saved = credentials::Store::load(&store).await?;
    if saved.is_none()
        || args.username.is_some()
        || args.password.is_some()
        || args.password_stdin
        || args.interactive
        || args.credential_store.is_some()
    {
        tracing::warn!(
            "{}",
            Message::ServiceInstallationRequiresSavedCredentialsRememberMe
        );
        login::run_with_wifi(client, paths, args, true, &wifi).await?;
    }
    service::install(paths).await
}
