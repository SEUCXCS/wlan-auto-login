//! Scoped credential selection and shared encoding; native APIs live under each platform.
use crate::infra::error::Message;
pub(crate) use crate::system::define::credentials::{CredentialMode, Store};
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

#[cfg(not(target_os = "macos"))]
use crate::infra::error::Context;
use crate::infra::error::Result;
use wlanlogin_lib::Credentials;
#[cfg(not(target_os = "macos"))]
use zeroize::Zeroizing;

pub struct ScopedStore {
    machine: Option<machine::MachineStore>,
    paths: crate::infra::paths::Paths,
}

impl ScopedStore {
    pub fn new(paths: &crate::infra::paths::Paths, mode: Option<CredentialMode>) -> Result<Self> {
        if paths.scope == crate::infra::scope::Scope::System {
            Ok(Self {
                paths: paths.clone(),
                machine: Some(machine::MachineStore::new(paths.clone(), mode)?),
            })
        } else {
            crate::infra::error::ensure!(
                mode.is_none(),
                Message::CredentialStoreIsOnlySupportedForSystem
            );
            Ok(Self {
                machine: None,
                paths: paths.clone(),
            })
        }
    }

    pub(crate) fn paths(&self) -> &crate::infra::paths::Paths {
        &self.paths
    }

    pub(crate) async fn load_background(
        &self,
        session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
    ) -> Result<crate::system::define::credentials::BackgroundRead> {
        use crate::system::define::credentials::BackgroundRead;
        let permit = validation::snapshot(&self.paths)?;
        // System storage never depends on a user's graphical session.
        if let Some(store) = &self.machine {
            return store
                .load(true)
                .await
                .map(|saved| BackgroundRead::from_saved(saved, permit))
                .map_err(|error| {
                    diagnostics::at(error, crate::infra::error::credentials::Stage::Read)
                });
        }

        #[cfg(target_os = "linux")]
        return user::load_for_daemon(session, permit).await;
        #[cfg(not(target_os = "linux"))]
        {
            let _ = session;
            user::load_for_daemon()
                .await
                .map(|saved| BackgroundRead::from_saved(saved, permit))
                .map_err(|error| {
                    diagnostics::at(error, crate::infra::error::credentials::Stage::Read)
                })
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

#[cfg(not(target_os = "macos"))]
use crate::infra::identity::{APP_ID as USER_SERVICE, CREDENTIAL_ENTRY as USER_ENTRY};
#[cfg(not(target_os = "macos"))]
pub async fn load_user() -> Result<Option<Credentials>> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(USER_SERVICE, USER_ENTRY)
            .context(Message::FailedToOpenSystemKeyring)
            .map_err(|error| {
                diagnostics::at(error, crate::infra::error::credentials::Stage::Connect)
            })?;
        let text = match entry.get_password() {
            Ok(value) => Zeroizing::new(value),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => {
                return Err(diagnostics::at(
                    e.into(),
                    crate::infra::error::credentials::Stage::Read,
                ));
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

mod codec;
pub(crate) use codec::{decode, encode};

pub(crate) mod diagnostics;
pub(crate) mod validation;
