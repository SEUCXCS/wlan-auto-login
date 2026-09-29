use super::{decode, encode};
use crate::infra::error::Message;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::infra::error::bail;
use crate::infra::error::{Context, Result};
use cxxy_wlanlogin_lib::Credentials;
use zeroize::Zeroizing;

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin";
const ENTRY: &str = "saved-login";

pub async fn delete() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "linux")]
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
        #[cfg(not(target_os = "linux"))]
        match keyring::Entry::new(SERVICE, ENTRY)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context(Message::FailedToDeleteUserCredentials),
        }
    })
    .await
    .context(Message::CredentialDeletionTaskFailed)?
}

pub async fn load() -> Result<Option<Credentials>> {
    tokio::task::spawn_blocking(|| {
        let entry =
            keyring::Entry::new(SERVICE, ENTRY).context(Message::FailedToOpenSystemKeyring)?;
        let text = match entry.get_password() {
            Ok(value) => Zeroizing::new(value),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => {
                return Err(e).context(Message::FailedToReadCredentialsUnlockTheSystemKeyring);
            }
        };
        decode(&text).map(Some)
    })
    .await
    .context(Message::CredentialReadTaskFailed)?
}

/// Background reads never ask the user to unlock the keychain.
pub async fn load_for_daemon() -> Result<Option<Credentials>> {
    #[cfg(target_os = "linux")]
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
    #[cfg(not(target_os = "linux"))]
    load().await
}

pub fn prepare_daemon() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: daemon is a separate process and performs no interactive
        // keychain operations. This process-wide switch prevents native dialogs.
        let status =
            unsafe { security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0) };
        if status != 0 {
            bail!(Message::FailedToDisableBackgroundKeychainPromptsStatus {
                status: (status).to_string()
            });
        }
    }
    Ok(())
}

pub async fn save(credentials: Credentials) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let text = encode(&credentials)?;
        keyring::Entry::new(SERVICE, ENTRY)
            .context(Message::FailedToOpenSystemKeyring)?
            .set_password(&text)
            .context(Message::FailedToSaveCredentialsUnlockTheSystemKeyring)?;
        Ok(())
    })
    .await
    .context(Message::CredentialSaveTaskFailed)?
}
