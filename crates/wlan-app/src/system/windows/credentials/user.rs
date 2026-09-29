use crate::infra::error::Message;

use crate::infra::error::{Context, Result};
pub(crate) use crate::system::shared::credentials::{load_user as load, save_user as save};
use cxxy_wlanlogin_lib::Credentials;

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin";
const ENTRY: &str = "saved-login";

pub async fn delete() -> Result<()> {
    tokio::task::spawn_blocking(
        || match keyring::Entry::new(SERVICE, ENTRY)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context(Message::FailedToDeleteUserCredentials),
        },
    )
    .await
    .context(Message::CredentialDeletionTaskFailed)?
}

/// Background reads never ask the user to unlock the keychain.
pub async fn load_for_daemon() -> Result<Option<Credentials>> {
    load().await
}

pub fn prepare_daemon() -> Result<()> {
    Ok(())
}
