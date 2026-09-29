//! Scoped credential selection and shared encoding; native APIs live under each platform.
use crate::infra::error::Message;
pub(crate) use crate::system::define::credentials::{CredentialMode, Store};
#[cfg(target_os = "macos")]
pub(crate) use crate::system::native::credentials::machine::authorize_binary;
#[cfg(target_os = "linux")]
pub(crate) use crate::system::native::credentials::machine::linux_credential_directive;
pub(crate) use crate::system::native::credentials::user::prepare_daemon;
use crate::system::native::credentials::{machine, user};
/// Delete without loading or decoding secrets, including damaged or duplicate files.
pub async fn delete(paths: &crate::infra::paths::Paths) -> crate::infra::error::Result<()> {
    match paths.scope {
        crate::infra::scope::Scope::System => machine::delete(paths).await,
        crate::infra::scope::Scope::User => user::delete().await,
    }
}

use crate::infra::error::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use wlanlogin_lib::Credentials;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub struct ScopedStore {
    machine: Option<machine::MachineStore>,
}

impl ScopedStore {
    pub fn new(paths: &crate::infra::paths::Paths, mode: Option<CredentialMode>) -> Result<Self> {
        if paths.scope == crate::infra::scope::Scope::System {
            Ok(Self {
                machine: Some(machine::MachineStore::new(paths.clone(), mode)?),
            })
        } else {
            crate::infra::error::ensure!(
                mode.is_none(),
                Message::CredentialStoreIsOnlySupportedForSystem
            );
            Ok(Self { machine: None })
        }
    }

    pub(crate) async fn load_background(&self) -> Result<Option<Credentials>> {
        match &self.machine {
            Some(store) => store.load(true).await,
            None => user::load_for_daemon().await,
        }
    }
}

impl Store for ScopedStore {
    async fn load(&self) -> Result<Option<Credentials>> {
        match &self.machine {
            Some(store) => store.load(false).await,
            None => user::load().await,
        }
    }

    async fn save(&self, credentials: Credentials) -> Result<()> {
        match &self.machine {
            Some(store) => store.save(credentials).await,
            None => user::save(credentials).await,
        }
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct Secret {
    version: u8,
    username: String,
    password: String,
}

pub(crate) fn encode(credentials: &Credentials) -> Result<Zeroizing<String>> {
    crate::infra::error::ensure!(
        !credentials.username.trim().is_empty() && !credentials.password.is_empty(),
        Message::UsernameAndPasswordMustNotBeEmpty
    );
    Ok(Zeroizing::new(serde_json::to_string(&Secret {
        version: 1,
        username: credentials.username.clone(),
        password: credentials.password.clone(),
    })?))
}

pub(crate) fn decode(text: &str) -> Result<Credentials> {
    let secret: Secret =
        serde_json::from_str(text).context(Message::InvalidSavedCredentialFormat)?;
    if secret.version != 1 || secret.username.trim().is_empty() || secret.password.is_empty() {
        bail!(Message::SavedCredentialsAreIncompleteRunLoginRememberMe);
    }
    Ok(Credentials {
        username: secret.username.clone(),
        password: secret.password.clone(),
    })
}

#[cfg(not(target_os = "macos"))]
use crate::infra::identity::{APP_ID as USER_SERVICE, CREDENTIAL_ENTRY as USER_ENTRY};
#[cfg(not(target_os = "macos"))]
pub async fn load_user() -> Result<Option<Credentials>> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(USER_SERVICE, USER_ENTRY)
            .context(Message::FailedToOpenSystemKeyring)?;
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
#[cfg(not(target_os = "macos"))]
pub async fn save_user(credentials: Credentials) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let text = encode(&credentials)?;
        keyring::Entry::new(USER_SERVICE, USER_ENTRY)
            .context(Message::FailedToOpenSystemKeyring)?
            .set_password(&text)
            .context(Message::FailedToSaveCredentialsUnlockTheSystemKeyring)?;
        Ok(())
    })
    .await
    .context(Message::CredentialSaveTaskFailed)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_or_unknown_saved_credentials() {
        assert!(decode(r#"{"version":1,"username":"student","password":""}"#).is_err());
        assert!(decode(r#"{"version":2,"username":"student","password":"secret"}"#).is_err());
        assert!(decode("not json").is_err());
    }
}
