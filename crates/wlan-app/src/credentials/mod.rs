//! Choose the credential backend for the selected scope.
mod background;
mod machine;
pub use machine::CredentialMode;
#[cfg(target_os = "macos")]
pub use machine::authorize_binary;
#[cfg(target_os = "linux")]
pub use machine::linux_credential_directive;

mod user;

pub(crate) use background::BackgroundReader;
pub(crate) use user::prepare_daemon;

/// Delete without loading or decoding secrets, including damaged or duplicate files.
pub async fn delete(paths: &crate::config::Paths) -> anyhow::Result<()> {
    match paths.scope {
        crate::scope::Scope::System => machine::delete(paths).await,
        crate::scope::Scope::User => user::delete().await,
    }
}

use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::Credentials;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub trait Store {
    fn load(&self) -> impl std::future::Future<Output = Result<Option<Credentials>>>;
    fn save(&self, credentials: Credentials) -> impl std::future::Future<Output = Result<()>>;
}

pub struct ScopedStore {
    machine: Option<machine::MachineStore>,
}

impl ScopedStore {
    pub fn new(paths: &crate::config::Paths, mode: Option<CredentialMode>) -> Result<Self> {
        if paths.scope == crate::scope::Scope::System {
            Ok(Self {
                machine: Some(machine::MachineStore::new(paths.clone(), mode)?),
            })
        } else {
            anyhow::ensure!(mode.is_none(), "--credential-store 只适用于系统凭据");
            Ok(Self { machine: None })
        }
    }

    async fn load_background(&self) -> Result<Option<Credentials>> {
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
struct Secret {
    version: u8,
    username: String,
    password: String,
}

fn encode(credentials: &Credentials) -> Result<Zeroizing<String>> {
    anyhow::ensure!(
        !credentials.username.trim().is_empty() && !credentials.password.is_empty(),
        "账号和密码不能为空"
    );
    Ok(Zeroizing::new(serde_json::to_string(&Secret {
        version: 1,
        username: credentials.username.clone(),
        password: credentials.password.clone(),
    })?))
}

fn decode(text: &str) -> Result<Credentials> {
    let secret: Secret = serde_json::from_str(text).context("保存的凭据格式无效")?;
    if secret.version != 1 || secret.username.trim().is_empty() || secret.password.is_empty() {
        bail!("保存的凭据不完整，请重新执行 login --remember-me");
    }
    Ok(Credentials {
        username: secret.username.clone(),
        password: secret.password.clone(),
    })
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
