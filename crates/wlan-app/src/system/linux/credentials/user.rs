use crate::infra::error::Message;
use crate::system::shared::credentials::decode;
use zeroize::Zeroizing;

use crate::infra::error::bail;
use crate::infra::error::{Context, Result};
pub(crate) use crate::system::shared::credentials::{load_user as load, save_user as save};
use wlanlogin_lib::Credentials;

use crate::infra::identity::{APP_ID as SERVICE, CREDENTIAL_ENTRY as ENTRY};

pub async fn delete() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        {
            use dbus_secret_service::{EncryptionType, SecretService};
            let service = SecretService::connect(EncryptionType::Dh)?;
            let items = service.search_items(std::collections::HashMap::from([
                ("service", SERVICE),
                ("username", ENTRY),
                ("target", "default"),
            ]))?;
            // Remove every matching application item, including duplicates, without
            // reading secrets or touching the collection or other applications.
            for item in items.unlocked.iter().chain(&items.locked) {
                item.delete()
                    .context(Message::FailedToDeleteUserCredentialsUnlockTheSystem)?;
            }
            Ok(())
        }
    })
    .await
    .context(Message::CredentialDeletionTaskFailed)?
}

/// Background reads never ask the user to unlock the keychain.
pub async fn load_for_daemon() -> Result<Option<Credentials>> {
    {
        tokio::task::spawn_blocking(|| {
            use dbus_secret_service::{EncryptionType, SecretService};
            let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 0)?;
            let items = service.search_items(std::collections::HashMap::from([
                ("service", SERVICE),
                ("username", ENTRY),
                ("target", "default"),
            ]))?;
            if !items.locked.is_empty() {
                bail!(Message::KeyringIsLockedRetryAfterUnlocking);
            }
            if items.unlocked.len() > 1 {
                bail!(Message::DuplicateSavedCredentialsFoundRemoveDuplicateEntriesFrom);
            }
            match items.unlocked.first() {
                Some(item) => {
                    let bytes = Zeroizing::new(item.get_secret()?);
                    decode(
                        std::str::from_utf8(&bytes).context(Message::SavedCredentialsAreNotUTF8)?,
                    )
                    .map(Some)
                }
                None => Ok(None),
            }
        })
        .await
        .context(Message::BackgroundCredentialReadTaskFailed)?
    }
}

pub fn prepare_daemon() -> Result<()> {
    Ok(())
}
